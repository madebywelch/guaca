//! A guacad container on the Docker daemon this process talks to, and the one
//! sequence that replaces it. Never starts an agent in this process.
//!
//! Two managers use it. The desktop manages On this Mac, and `guaca-updater`
//! manages the host on a box it runs beside (`updater.rs`). They differ in how
//! the container is made, which is a [`Spec`], and in where an update comes
//! from, which is a [`Target`], and in nothing else: the backup before a
//! replacement and the restore after a failed one are the same code for both.
//! The restore is automatic because the person who pressed the button cannot
//! be assumed to have a terminal, on either.
//!
//! Desktop container names are scoped to the bundle ID; volumes survive app
//! upgrades.
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::process::Command;
use tokio::sync::Mutex;

/// The host this app installs. Unpinned, it is the published tag of this
/// app's own version: a spelled-out tag stayed at 0.1.0 after the app moved
/// on, and offered every source build a downgrade of the host it managed.
pub const IMAGE: &str = match option_env!("GUACA_BACKEND_IMAGE") {
    Some(image) => image,
    None => concat!("ghcr.io/madebywelch/guaca/guacad:", env!("CARGO_PKG_VERSION")),
};

/// Where a box's updater listens, as both containers see it.
pub const UPDATER_SOCKET: &str = "/run/guaca-updater/updater.sock";
const UPDATER_DIR: &str = "/run/guaca-updater";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub origin: String,
    pub token: String,
}

/// How a host container is made. The desktop and a box disagree about
/// nothing else.
#[derive(Clone)]
pub struct Spec {
    pub name: String,
    /// The label that says this manager made the container. One with the name
    /// and without the label is somebody else's, and is never touched.
    pub owner: &'static str,
    /// The workspace, mounted at `/var/lib/guaca`.
    pub volume: String,
    /// The loopback port it is published on. `None` takes any free one.
    pub port: Option<u16>,
    /// Variables handed from this process's environment, by name, so a value
    /// never appears in an argument list.
    pub env: &'static [&'static str],
    /// The volume holding the updater's socket, when there is an updater.
    pub socket: Option<String>,
    pub reach: Reach,
}

/// Where the manager stands when it asks the host whether it is ready.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// On the machine the port is published on, which is the desktop.
    Published,
    /// In another container on the same Docker network, which is the updater.
    /// A box publishes to its own loopback, which a container cannot see.
    Bridge,
}

/// What an update installs, and what the host must say about itself after.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub image: String,
    pub version: String,
    pub api_generation: u32,
}

/// Where an update is. The strings are what the operator reads and what the
/// journal has always held, so a journal an older build wrote still reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Downloading,
    Stopping,
    BackingUp,
    Starting,
    Verifying,
    Updated,
    Canceled,
    Restoring,
    Restored,
    RecoveryNeeded,
}

impl Stage {
    const ALL: [Stage; 10] = [
        Stage::Downloading,
        Stage::Stopping,
        Stage::BackingUp,
        Stage::Starting,
        Stage::Verifying,
        Stage::Updated,
        Stage::Canceled,
        Stage::Restoring,
        Stage::Restored,
        Stage::RecoveryNeeded,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Downloading => "Downloading update",
            Stage::Stopping => "Stopping host",
            Stage::BackingUp => "Backing up workspace",
            Stage::Starting => "Starting updated host",
            Stage::Verifying => "Verifying host",
            Stage::Updated => "Host updated",
            Stage::Canceled => "Update canceled",
            Stage::Restoring => "Restoring previous version",
            Stage::Restored => "Previous version restored",
            Stage::RecoveryNeeded => "Recovery needed",
        }
    }

    /// Whether the operation is over, one way or the other.
    pub fn finished(self) -> bool {
        matches!(self, Stage::Updated | Stage::Canceled | Stage::Restored | Stage::RecoveryNeeded)
    }
}

impl Serialize for Stage {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Stage {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let said = String::deserialize(deserializer)?;
        Stage::ALL
            .into_iter()
            .find(|stage| stage.as_str() == said)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown update stage {said}")))
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Operation {
    pub stage: Stage,
    pub backup: Option<String>,
    pub previous_image: String,
    pub target_image: String,
    /// What the host has to report after the swap. Absent from a journal an
    /// older build wrote, which is then restored rather than trusted.
    #[serde(default)]
    pub target_version: String,
    #[serde(default)]
    pub api_generation: u32,
    /// The port it was published on, so a recovery publishes the same one.
    #[serde(default)]
    pub port: Option<u16>,
    /// Where the workspace a failed update left was copied before the backup
    /// was restored over it.
    #[serde(default)]
    pub preserved: Option<String>,
    pub error: Option<String>,
}

impl Operation {
    fn target(&self) -> Option<Target> {
        (!self.target_version.is_empty() && self.api_generation > 0).then(|| Target {
            image: self.target_image.clone(),
            version: self.target_version.clone(),
            api_generation: self.api_generation,
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub state: &'static str,
    pub message: String,
    pub update_available: bool,
    pub updating: bool,
    pub origin: Option<String>,
    pub target_image: String,
    pub target_version: String,
    pub operation: Option<Operation>,
}

/// The container that is running, as the updater reports it.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Running {
    pub image: String,
    pub version: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExistingHost {
    pub name: String,
    pub label: String,
    pub origin: String,
}

pub struct LocalHost {
    spec: Spec,
    /// What a container is started from when there is none.
    image: String,
    lock: Mutex<()>,
    journal: Option<PathBuf>,
    binary: PathBuf,
}

struct ProcessLock(std::fs::File);

impl Drop for ProcessLock {
    fn drop(&mut self) {
        // A concurrent fork can keep a duplicate descriptor alive until exec.
        // Closing our descriptor alone would leave that child holding the lock.
        if let Err(error) = self.0.unlock() {
            tracing::warn!(%error, "could not release the host operation lock");
        }
    }
}

pub fn docker_binary() -> PathBuf {
    std::env::var_os("PATH")
        .and_then(|p| std::env::split_paths(&p).map(|p| p.join("docker")).find(|p| p.is_file()))
        .or_else(|| {
            let p =
                std::path::PathBuf::from("/Applications/Docker.app/Contents/Resources/bin/docker");
            p.is_file().then_some(p)
        })
        .unwrap_or_else(|| "docker".into())
}

pub(crate) async fn run_docker(
    binary: &Path,
    args: &[&str],
    seconds: u64,
) -> Result<String, String> {
    let mut command = Command::new(binary);
    command.args(args).kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(seconds), command.output())
        .await
        .map_err(|_| "Docker took too long to answer. Check Docker and try again.".to_string())?
        .map_err(|_| {
            "Docker is not installed. Install Docker Desktop, then check again.".to_string()
        })?;
    if !output.status.success() {
        // Arguments never contain credentials. Avoid dumping Docker's environment or logs.
        return Err(format!(
            "Docker could not finish: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

impl LocalHost {
    async fn docker(&self, args: &[&str], seconds: u64) -> Result<String, String> {
        run_docker(&self.binary, args, seconds).await
    }

    /// On this Mac: a container per bundle ID, on any free loopback port.
    pub fn new(identifier: &str, image: &str) -> Self {
        let name: String =
            identifier.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
        let name = format!("{name}-host");
        Self::with_spec(
            Spec {
                volume: format!("{name}-data"),
                name,
                owner: "bot.guaca.desktop",
                port: None,
                env: &[],
                socket: None,
                reach: Reach::Published,
            },
            image,
        )
    }

    pub fn with_spec(spec: Spec, image: &str) -> Self {
        Self {
            spec,
            image: image.to_string(),
            lock: Mutex::new(()),
            journal: None,
            binary: docker_binary(),
        }
    }

    /// The update a desktop installs: the host built with it.
    pub fn bundled(&self) -> Target {
        Target {
            image: self.image.clone(),
            version: env!("CARGO_PKG_VERSION").into(),
            api_generation: crate::updates::protocol().generation,
        }
    }

    async fn inspect(&self) -> Result<Option<Value>, String> {
        owned(&self.binary, &self.spec.name, (self.spec.owner, &self.spec.name)).await
    }

    async fn available(&self) -> Result<(), String> {
        // A selected remote Docker context would create a host on a different machine.
        let endpoint = self
            .docker(&["context", "inspect", "--format", "{{.Endpoints.docker.Host}}"], 10)
            .await?;
        let endpoint = if std::env::var("DOCKER_CONTEXT").is_ok_and(|s| !s.is_empty()) {
            endpoint
        } else {
            std::env::var("DOCKER_HOST").unwrap_or(endpoint)
        };
        if !endpoint.starts_with("unix://") && !endpoint.starts_with("npipe://") {
            return Err("Docker is connected to another computer. Select a local Docker context for On this Mac.".into());
        }
        let os = self.docker(&["info", "--format", "{{.OSType}}"], 15).await
            .map_err(|_| "Docker is installed but is not ready. Open Docker Desktop, wait for it to start, then check again.".to_string())?;
        if os != "linux" {
            return Err("Guaca needs Docker's Linux containers. Switch Docker to Linux containers and check again.".into());
        }
        Ok(())
    }

    pub async fn existing(&self) -> Result<Vec<ExistingHost>, String> {
        self.available().await?;
        let names = self
            .docker(
                &[
                    "container",
                    "ls",
                    "--filter",
                    "label=com.docker.compose.service=guacad",
                    "--format",
                    "{{.Names}}",
                ],
                15,
            )
            .await?;
        let mut hosts = Vec::new();
        for name in names.lines() {
            let raw = self.docker(&["container", "inspect", name], 15).await?;
            let values: Vec<Value> =
                serde_json::from_str(&raw).map_err(|_| "Docker returned an unreadable host.")?;
            if let Some(value) = values.first() {
                if let Ok(port) = published_port(value) {
                    hosts.push(ExistingHost {
                        name: name.into(),
                        label: value["Config"]["Labels"]["com.docker.compose.project"]
                            .as_str()
                            .unwrap_or(name)
                            .into(),
                        origin: format!("http://127.0.0.1:{port}"),
                    });
                }
            }
        }
        Ok(hosts)
    }

    pub async fn connect_existing(&self, name: &str) -> Result<Connection, String> {
        let host = self
            .existing()
            .await?
            .into_iter()
            .find(|h| h.name == name)
            .ok_or("That local Guaca host is no longer available.")?;
        // No caller-supplied path or command can reach Docker through this API.
        let token =
            self.docker(&["exec", &host.name, "cat", "/var/lib/guaca/config/token"], 15).await?;
        if token.is_empty() {
            return Err("This host has no saved access key. Connect with its address and key under Remote host.".into());
        }
        Ok(Connection { origin: host.origin, token })
    }

    pub fn with_journal(mut self, path: PathBuf) -> Self {
        self.journal = Some(path);
        self
    }

    /// The `docker` this manager runs, for a caller managing a container
    /// beside this one.
    #[cfg(feature = "server")]
    pub(crate) fn docker_path(&self) -> &Path {
        &self.binary
    }

    #[cfg(all(test, feature = "server"))]
    pub(crate) fn with_docker(mut self, binary: PathBuf) -> Self {
        self.binary = binary;
        self
    }

    pub fn is_updating(&self) -> bool {
        self.lock.try_lock().is_err()
    }

    pub fn operation(&self) -> Result<Option<Operation>, String> {
        let Some(path) = &self.journal else {
            return Ok(None);
        };
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map(Some)
                .map_err(|_| "The host update journal is unreadable. Review the Docker host and backups before retrying.".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("Could not read host update progress: {e}")),
        }
    }

    fn record(&self, operation: &Operation) -> Result<(), String> {
        if let Some(path) = &self.journal {
            write_journal(path, operation)?;
        }
        tracing::info!(
            container = %self.spec.name,
            stage = operation.stage.as_str(),
            backup = ?operation.backup,
            preserved = ?operation.preserved,
            "host update"
        );
        Ok(())
    }

    /// Why an operation that was stopped midway cannot be settled here, if it
    /// cannot. Read without Docker, for a status that must not block.
    fn stuck(&self) -> Result<(), String> {
        match self.operation()? {
            Some(op) if op.stage == Stage::RecoveryNeeded => Err(format!(
                "The last host update could not be undone. The workspace from before it is saved in Docker volume {}. Review the update instructions before starting or replacing this host.",
                op.backup.as_deref().unwrap_or("with this host's name and -backup-")
            )),
            _ => Ok(()),
        }
    }

    /// Finishes or undoes an operation a crash left midway, before anything
    /// else touches the container. Holds the caller's locks.
    ///
    /// Before the swap the old container was never removed, so it is started
    /// again. After it, the new one is kept only if it passes the same checks
    /// an uninterrupted update makes, which is what separates a host that
    /// finished updating from one that died doing it. Anything else is
    /// restored from the backup the operation recorded before it could touch
    /// the workspace.
    async fn settle(&self) -> Result<(), String> {
        self.stuck()?;
        let Some(mut op) = self.operation()? else {
            return Ok(());
        };
        match op.stage {
            Stage::Updated | Stage::Canceled | Stage::Restored | Stage::RecoveryNeeded => Ok(()),
            Stage::Downloading | Stage::Stopping | Stage::BackingUp => {
                op.stage = Stage::Canceled;
                op.error = Some(
                    "The update was interrupted before the host was replaced. The previous host was kept."
                        .into(),
                );
                self.record(&op)?;
                if self.inspect().await?.is_some() {
                    self.docker(&["start", &self.spec.name], 60).await?;
                }
                Ok(())
            }
            Stage::Starting | Stage::Verifying => {
                if let Some(target) = op.target() {
                    if self.finished(&target).await.is_ok() {
                        op.stage = Stage::Updated;
                        return self.record(&op);
                    }
                }
                op.error = Some("The update was interrupted while the new version started.".into());
                self.restore(&mut op).await
            }
            Stage::Restoring => self.restore(&mut op).await,
        }
    }

    /// Whether the container is the target, running, and answering as it.
    async fn finished(&self, target: &Target) -> Result<(), String> {
        let value = self.inspect().await?.ok_or("The host container is missing.")?;
        if value["Config"]["Image"].as_str() != Some(&target.image) {
            return Err("The host is not running the update.".into());
        }
        let connection = self.connection(&value).await?;
        self.verify(&connection, &target.version, target.api_generation, "").await
    }

    /// The desktop's view: whether Docker is ready and whether the host this
    /// app bundles differs from the one running.
    pub async fn status(&self) -> Status {
        let mut status = Status {
            state: "unavailable",
            message: String::new(),
            update_available: false,
            updating: self.is_updating(),
            origin: None,
            target_image: self.image.clone(),
            target_version: env!("CARGO_PKG_VERSION").into(),
            operation: self.operation().ok().flatten(),
        };
        if let Err(message) = self.available().await {
            status.state =
                if message.contains("not installed") { "missing" } else { "unavailable" };
            status.message = message;
            return status;
        }
        match self.inspect().await {
            Ok(Some(value)) => {
                let running = value["State"]["Running"] == true;
                status.state = if running { "running" } else { "stopped" };
                status.message = if running {
                    "Docker is running. Your local host is available."
                } else {
                    "Docker is ready. Your local host is stopped."
                }
                .into();
                status.origin =
                    published_port(&value).ok().map(|p| format!("http://127.0.0.1:{p}"));
                let newer = newer_than(&value, env!("CARGO_PKG_VERSION"));
                status.update_available = value["Config"]["Image"].as_str() != Some(&self.image)
                    && !newer
                    && self.stuck().is_ok();
                if let Err(error) = self.stuck() {
                    status.message = error;
                }
                if newer {
                    status.message = "This host is newer than this desktop app. Update Guaca before managing it.".into();
                }
            }
            Ok(None) => {
                status.state = "ready";
                status.message = "Docker is ready. Guaca can set up your local host.".into();
            }
            Err(message) => {
                status.message = message;
            }
        }
        status
    }

    /// The image and release of the container that is there, if one is.
    pub async fn running(&self) -> Result<Option<Running>, String> {
        Ok(self.inspect().await?.map(|value| Running {
            image: value["Config"]["Image"].as_str().unwrap_or_default().into(),
            version: value["Config"]["Labels"]["org.opencontainers.image.version"]
                .as_str()
                .filter(|v| !v.is_empty())
                .map(Into::into),
        }))
    }

    fn process_lock(&self) -> Result<Option<ProcessLock>, String> {
        let Some(journal) = &self.journal else {
            return Ok(None);
        };
        if let Some(parent) = journal.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Could not create host update directory: {e}"))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(journal.with_extension("lock"))
            .map_err(|e| format!("Could not lock the host update: {e}"))?;
        file.try_lock()
            .map_err(|_| "Another Guaca process is managing this host. Wait for it to finish.")?;
        Ok(Some(ProcessLock(file)))
    }

    pub async fn start(&self) -> Result<Connection, String> {
        let _lock = self.lock.lock().await;
        let _process_lock = self.process_lock()?;
        self.settle().await?;
        self.start_unlocked(&self.image, None).await
    }

    async fn image_ready(&self, image: &str) -> Result<(), String> {
        if self.docker(&["image", "inspect", image], 15).await.is_err() {
            self.docker(&["pull", image], 900).await.map_err(|error| {
                tracing::warn!(%image, %error, "could not download the host image");
                pull_failure(image, &error)
            })?;
        }
        Ok(())
    }

    /// Update is explicit: download before interrupting work, then preserve a
    /// complete stopped-volume backup before the new binary can migrate it.
    /// A failure after the swap restores that backup and the previous image,
    /// and the error says so.
    pub async fn update(
        &self,
        target: &Target,
        expected_origin: Option<&str>,
    ) -> Result<Connection, String> {
        let _lock = self
            .lock
            .try_lock()
            .map_err(|_| "A host operation is already running. Wait for it to finish.")?;
        let _process_lock = self.process_lock()?;
        self.settle().await?;
        let mut op = Operation {
            stage: Stage::Downloading,
            backup: None,
            previous_image: String::new(),
            target_image: target.image.clone(),
            target_version: target.version.clone(),
            api_generation: target.api_generation,
            port: None,
            preserved: None,
            error: None,
        };
        let result = match self.admit(target, expected_origin, &mut op).await {
            Ok(port) => {
                self.record(&op)?;
                self.replace(&mut op, port, target).await
            }
            Err(error) => Err(error),
        };
        let Err(error) = result else {
            return result;
        };
        op.error = Some(error.clone());
        if matches!(op.stage, Stage::Starting | Stage::Verifying) {
            return match self.restore(&mut op).await {
                Ok(()) => Err(format!(
                    "{error} Guaca put the previous version back, with the workspace as it was when the update began."
                )),
                Err(restore) => Err(restore),
            };
        }
        op.stage = Stage::Canceled;
        if let Err(record_error) = self.record(&op) {
            tracing::error!(%record_error, %error, "could not record update failure");
        }
        Err(error)
    }

    /// Everything checked before an update may interrupt anything. Returns
    /// the port the replacement must keep.
    async fn admit(
        &self,
        target: &Target,
        expected_origin: Option<&str>,
        op: &mut Operation,
    ) -> Result<u16, String> {
        self.available().await?;
        let old = self
            .inspect()
            .await?
            .ok_or("The connected local host no longer exists. Reconnect before updating.")?;
        op.previous_image = old["Config"]["Image"].as_str().unwrap_or_default().into();
        let correct_volume = old["Mounts"].as_array().is_some_and(|mounts| {
            mounts.iter().any(|mount| {
                mount["Destination"] == "/var/lib/guaca"
                    && mount["Type"] == "volume"
                    && mount["Name"] == self.spec.volume
            })
        });
        if !correct_volume {
            return Err("This container uses a different workspace volume than the one Guaca manages. It was left untouched. Update it using the self-hosted instructions.".into());
        }
        let port = published_port(&old)?;
        op.port = Some(port);
        if expected_origin.is_some_and(|origin| origin != format!("http://127.0.0.1:{port}")) {
            return Err("The selected workspace is not the local host this app manages. Reconnect before updating.".into());
        }
        if newer_than(&old, &target.version) {
            return Err(format!(
                "This host is newer than Guaca {}. Update Guaca first.",
                target.version
            ));
        }
        if op.previous_image == target.image {
            return Err(format!("This host already runs Guaca {}.", target.version));
        }
        Ok(port)
    }

    async fn replace(
        &self,
        op: &mut Operation,
        port: u16,
        target: &Target,
    ) -> Result<Connection, String> {
        self.image_ready(&target.image).await?;
        let raw = self.docker(&["image", "inspect", &target.image], 15).await?;
        let image: Vec<Value> =
            serde_json::from_str(&raw).map_err(|_| "Docker returned unreadable image metadata.")?;
        let labels = image.first().map(|v| &v["Config"]["Labels"]);
        let revision = labels
            .and_then(|l| l["org.opencontainers.image.revision"].as_str())
            .unwrap_or_default()
            .to_string();
        // Checked while the old host still runs. After the swap the database
        // is either migrated by a binary nobody vouched for or refused by an
        // older one, and both end in a restore.
        let labeled =
            labels.and_then(|l| l["org.opencontainers.image.version"].as_str()).unwrap_or_default();
        if !labeled.is_empty() && labeled != target.version {
            return Err(format!(
                "The host image reports version {labeled}, not {}. The running host was left untouched. Reinstall Guaca so it installs the host built with it.",
                target.version
            ));
        }
        op.stage = Stage::Stopping;
        self.record(op)?;
        self.docker(&["stop", &self.spec.name], 60).await?;
        let backup = format!("{}-backup-{}", self.spec.name, uuid::Uuid::new_v4());
        op.backup = Some(backup.clone());
        op.stage = Stage::BackingUp;
        // If journaling fails after stopping, restart the old host as with a
        // failed copy. The target has not mounted the live workspace yet.
        let saved = match self.record(op) {
            Err(e) => Err(e),
            Ok(()) => self.copy(&target.image, &self.spec.volume, &backup).await,
        };
        if saved.is_err() {
            return match self.docker(&["start", &self.spec.name], 60).await {
                Ok(_) => Err("The host backup could not be completed. The update was canceled and the previous host was restarted.".into()),
                Err(_) => Err("The backup failed and Docker could not restart the previous host. Your data is preserved. Check Docker before trying again.".into()),
            };
        }
        // Persist the recovery point before any destructive action. A manager
        // killed after this write restores rather than guesses.
        op.stage = Stage::Starting;
        self.record(op)?;
        // One backup is kept: this one, which is what a failed update restores
        // and what the operator would roll back to. Every earlier backup, and
        // every copy a failed update left, is of a state this host has moved
        // past. Removed only now that this one is recorded, so there is never
        // a moment with none.
        self.prune(&backup).await;
        self.docker(&["rm", &self.spec.name], 30).await?;
        let connection = self.start_unlocked(&target.image, Some(port)).await?;
        op.stage = Stage::Verifying;
        self.record(op)?;
        self.verify(&connection, &target.version, target.api_generation, &revision).await?;
        op.stage = Stage::Updated;
        op.error = None;
        self.record(op)?;
        Ok(connection)
    }

    /// Asks the host who it is, then asks it something only its workspace
    /// can answer with its key. An empty revision is not compared.
    async fn verify(
        &self,
        connection: &Connection,
        version: &str,
        api_generation: u32,
        revision: &str,
    ) -> Result<(), String> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| e.to_string())?;
        let health: Value = http
            .get(format!("{}/health", connection.origin))
            .send()
            .await
            .map_err(|e| format!("The updated host did not answer: {e}."))?
            .json()
            .await
            .map_err(|_| "The updated host returned unreadable health information.")?;
        if (!revision.is_empty() && health["build"].as_str() != Some(revision))
            || health["version"] != version
            || health["apiGeneration"] != api_generation
        {
            return Err(format!(
                "The updated host reported {} (API {}), not Guaca {version}.",
                health["version"].as_str().unwrap_or("no version"),
                health["apiGeneration"]
            ));
        }
        self.admitted(&http, connection).await
    }

    async fn admitted(
        &self,
        http: &reqwest::Client,
        connection: &Connection,
    ) -> Result<(), String> {
        let result: Value = http
            .post(format!("{}/v1/call", connection.origin))
            .bearer_auth(&connection.token)
            .json(&serde_json::json!({"name":"capabilities","args":{}}))
            .send()
            .await
            .map_err(|_| "The updated workspace could not be reached.")?
            .json()
            .await
            .map_err(|_| "The updated workspace returned an unreadable answer.")?;
        if !result["ok"].is_object() {
            return Err("The updated workspace did not accept its access key.".into());
        }
        Ok(())
    }

    /// Puts the workspace back as it was when the update began and runs the
    /// image that was running then.
    ///
    /// The workspace the failed version left is copied aside first and kept,
    /// because a restore is a decision about somebody's data made without
    /// asking them. The volume keeps its name: a restored host that mounted
    /// a new one would be a host its own manager no longer recognizes.
    async fn restore(&self, op: &mut Operation) -> Result<(), String> {
        op.stage = Stage::Restoring;
        self.record(op)?;
        let result = self.restore_unrecorded(op).await;
        op.stage = if result.is_ok() { Stage::Restored } else { Stage::RecoveryNeeded };
        if let Err(error) = &result {
            tracing::error!(%error, backup = ?op.backup, "could not restore the previous host");
            op.error = Some(format!(
                "{} The previous version could not be restored: {error} The workspace from before the update is saved in Docker volume {}.",
                op.error.as_deref().unwrap_or("The update failed."),
                op.backup.as_deref().unwrap_or("with this host's name and -backup-")
            ));
        }
        self.record(op)?;
        match result {
            Ok(()) => Ok(()),
            Err(_) => Err(op.error.clone().unwrap_or_default()),
        }
    }

    async fn restore_unrecorded(&self, op: &mut Operation) -> Result<(), String> {
        let backup = op.backup.clone().ok_or("The update recorded no backup.")?;
        if op.previous_image.is_empty() {
            return Err("The update recorded no previous image.".into());
        }
        let _ = self.docker(&["rm", "--force", &self.spec.name], 60).await;
        if self.inspect().await?.is_some() {
            return Err("Docker could not remove the failed host.".into());
        }
        // Either image carries `cp`; the target is the one certainly present,
        // since the swap only happens after it downloads.
        let tools = if op.target_image.is_empty() { &op.previous_image } else { &op.target_image };
        let preserved = format!("{}-failed-{}", self.spec.name, uuid::Uuid::new_v4());
        self.copy(tools, &self.spec.volume, &preserved).await?;
        op.preserved = Some(preserved);
        self.record(op)?;
        self.docker(
            &[
                "run",
                "--rm",
                "--user",
                "0",
                "--entrypoint",
                "sh",
                "--mount",
                &format!("type=volume,src={},dst=/data", self.spec.volume),
                "--mount",
                &format!("type=volume,src={backup},dst=/backup,readonly"),
                tools,
                "-c",
                "find /data -mindepth 1 -delete && cp -a /backup/. /data/",
            ],
            600,
        )
        .await?;
        let connection = self.start_unlocked(&op.previous_image, op.port).await?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| e.to_string())?;
        self.admitted(&http, &connection).await
    }

    /// A whole volume into a new one, as root, with `image` supplying `cp`.
    /// Removes this host's backups and failed-update copies, all but `keep`.
    /// One that will not go is left and said: the update does not depend on
    /// it, and the next update tries again.
    async fn prune(&self, keep: &str) {
        let names = match self.docker(&["volume", "ls", "--format", "{{.Name}}"], 30).await {
            Ok(names) => names,
            Err(error) => {
                tracing::warn!(%error, "could not list earlier backups; they were left");
                return;
            }
        };
        let ours = [format!("{}-backup-", self.spec.name), format!("{}-failed-", self.spec.name)];
        for name in names.lines().map(str::trim) {
            if name == keep || !ours.iter().any(|prefix| name.starts_with(prefix.as_str())) {
                continue;
            }
            match self.docker(&["volume", "rm", name], 60).await {
                Ok(_) => tracing::info!(volume = name, "removed an earlier backup"),
                Err(error) => {
                    tracing::warn!(volume = name, %error, "could not remove an earlier backup")
                }
            }
        }
    }

    async fn copy(&self, image: &str, from: &str, to: &str) -> Result<String, String> {
        self.docker(
            &[
                "run",
                "--rm",
                "--user",
                "0",
                "--entrypoint",
                "cp",
                "--mount",
                &format!("type=volume,src={from},dst=/source,readonly"),
                "--mount",
                &format!("type=volume,src={to},dst=/backup"),
                image,
                "-a",
                "/source/.",
                "/backup/",
            ],
            600,
        )
        .await
    }

    async fn start_unlocked(&self, image: &str, port: Option<u16>) -> Result<Connection, String> {
        self.available().await?;
        if self.inspect().await?.is_none() {
            self.image_ready(image).await?;
            self.create(image, port.or(self.spec.port)).await?;
        } else {
            self.docker(&["start", &self.spec.name], 60).await?;
        }
        let value = self.inspect().await?.ok_or("The local host disappeared while starting.")?;
        self.connection(&value).await
    }

    async fn create(&self, image: &str, port: Option<u16>) -> Result<(), String> {
        let binding =
            port.map(|p| format!("127.0.0.1:{p}:8787")).unwrap_or_else(|| "127.0.0.1::8787".into());
        let label = format!("{}={}", self.spec.owner, self.spec.name);
        let workspace = format!("type=volume,src={},dst=/var/lib/guaca", self.spec.volume);
        let mut args = vec![
            "run",
            "--detach",
            "--name",
            &self.spec.name,
            "--label",
            &label,
            "--init",
            "--restart",
            "unless-stopped",
            "--stop-timeout",
            "30",
            "--publish",
            &binding,
            "--mount",
            &workspace,
            "--add-host",
            "host.docker.internal:host-gateway",
        ];
        for name in self.spec.env {
            if std::env::var_os(name).is_some() {
                args.extend(["--env", *name]);
            }
        }
        let socket = self
            .spec
            .socket
            .as_ref()
            .map(|volume| format!("type=volume,src={volume},dst={UPDATER_DIR},readonly"));
        let socket_env = format!("GUACA_UPDATER_SOCKET={UPDATER_SOCKET}");
        if let Some(mount) = &socket {
            args.extend(["--mount", mount, "--env", &socket_env]);
        }
        args.push(image);
        self.docker(&args, 60).await.map(drop)
    }

    /// Waits for the container to answer as guacad, and reads its key.
    async fn connection(&self, value: &Value) -> Result<Connection, String> {
        let port = published_port(value)?;
        let published = format!("http://127.0.0.1:{port}");
        let probe = match self.spec.reach {
            Reach::Published => published.clone(),
            Reach::Bridge => format!("http://{}:8787", bridge_address(value)?),
        };
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|e| e.to_string())?;
        for _ in 0..60 {
            if let Ok(response) = http.get(format!("{probe}/health")).send().await {
                if response.status().is_success() {
                    let health: Value = response.json().await.unwrap_or_default();
                    if health["service"] == "guacad" {
                        let token = self.token().await?;
                        let origin =
                            if self.spec.reach == Reach::Published { published } else { probe };
                        return Ok(Connection { origin, token });
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        Err("The local host started but is not ready to connect. Check Docker's Guaca container for details, then try again. Your data is preserved.".into())
    }

    /// The key the host answers to. A box that was given one in its
    /// environment never writes it to the token file.
    async fn token(&self) -> Result<String, String> {
        if self.spec.env.contains(&"GUACA_TOKEN") {
            if let Some(token) = std::env::var("GUACA_TOKEN")
                .ok()
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
            {
                return Ok(token);
            }
        }
        let token = self
            .docker(&["exec", &self.spec.name, "cat", "/var/lib/guaca/config/token"], 10)
            .await?;
        if token.is_empty() {
            return Err("The local host has not created its access key. Try again.".into());
        }
        Ok(token)
    }
}

/// The container called `name`, if there is one and it carries the `label`
/// its maker gave it. One by that name without it is refused rather than
/// returned. The label is compared to what the container was made as, which
/// is its name until it is renamed aside.
pub(crate) async fn owned(
    binary: &Path,
    name: &str,
    label: (&str, &str),
) -> Result<Option<Value>, String> {
    // Listing first distinguishes an absent container from an unavailable daemon.
    let ids = run_docker(
        binary,
        &["container", "ls", "-a", "--filter", &format!("name=^/{name}$"), "--format", "{{.ID}}"],
        15,
    )
    .await?;
    if ids.is_empty() {
        return Ok(None);
    }
    let text = run_docker(binary, &["container", "inspect", name], 15).await?;
    let list: Vec<Value> = serde_json::from_str(&text)
        .map_err(|_| "Docker returned an unreadable container.".to_string())?;
    let value = list.into_iter().next().ok_or("Docker returned no container.")?;
    if value["Config"]["Labels"][label.0] != label.1 {
        return Err(
            "A different container is using Guaca's name. It has been left untouched.".into()
        );
    }
    Ok(Some(value))
}

/// Why the host image did not arrive, split where the operator's next step
/// splits. Every failure used to say "check your connection", including a
/// registry answering 403 for an image this build names but nobody published,
/// which no connection will fix.
fn pull_failure(image: &str, error: &str) -> String {
    let error = error.to_ascii_lowercase();
    if ["denied", "unauthorized", "manifest unknown", "not found"].iter().any(|s| error.contains(s))
    {
        format!("The registry refused Guaca's host image, {image}. It may not be published yet. Source installs can build it with scripts/install.sh.")
    } else {
        "The Guaca host could not be downloaded. Check your connection and try again. Source installs can build it with scripts/install.sh.".into()
    }
}

/// Whether the container's release is newer than `version`. A container
/// without a release label is a source build and is never ordered.
fn newer_than(value: &Value, version: &str) -> bool {
    let current = value["Config"]["Labels"]["org.opencontainers.image.version"]
        .as_str()
        .and_then(|s| semver::Version::parse(s).ok());
    let Ok(version) = semver::Version::parse(version) else {
        return false;
    };
    current.is_some_and(|v| v > version)
}

fn write_journal(path: &Path, op: &Operation) -> Result<(), String> {
    let write = || -> Result<(), Box<dyn std::error::Error>> {
        use std::io::Write;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp = path.with_extension("tmp");
        let mut options = OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(&serde_json::to_vec(op)?)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)?;
        #[cfg(unix)]
        if let Some(parent) = path.parent() {
            std::fs::File::open(parent)?.sync_all()?;
        }
        Ok(())
    };
    write().map_err(|e| format!("Could not save host update progress: {e}"))
}

fn published_port(value: &Value) -> Result<u16, String> {
    let ports = value["NetworkSettings"]["Ports"]["8787/tcp"]
        .as_array()
        .ok_or("Guaca has no local connection port.")?;
    let binding = ports
        .iter()
        .find(|p| p["HostIp"] == "127.0.0.1")
        .ok_or("Guaca's port is not restricted to this computer.")?;
    binding["HostPort"]
        .as_str()
        .and_then(|p| p.parse().ok())
        .filter(|p| *p > 0)
        .ok_or("Guaca has an invalid connection port.".into())
}

/// The container's address on a Docker network another container shares.
fn bridge_address(value: &Value) -> Result<String, String> {
    value["NetworkSettings"]["Networks"]
        .as_object()
        .and_then(|networks| {
            networks.values().find_map(|n| n["IPAddress"].as_str().filter(|ip| !ip.is_empty()))
        })
        .map(Into::into)
        .ok_or(
            "The host has no address on a Docker network. Check Docker's Guaca container.".into(),
        )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    async fn docker(args: &[&str], seconds: u64) -> Result<String, String> {
        run_docker(&docker_binary(), args, seconds).await
    }
    #[cfg(unix)]
    async fn simulated(
        failure: &str,
        version: &str,
    ) -> (LocalHost, tempfile::TempDir, tokio::task::JoinHandle<()>, String) {
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = socket.local_addr().unwrap();
        let version = version.to_string();
        let app = axum::Router::new()
            .route("/health", axum::routing::get(move || {
                let version = version.clone();
                async move { axum::Json(serde_json::json!({"service":"guacad", "build":"abcdef1", "version":version, "apiGeneration":1})) }
            }))
            .route("/v1/call", axum::routing::post(|| async {axum::Json(serde_json::json!({"ok":{}}))}));
        let task = tokio::spawn(async move {
            axum::serve(socket, app).await.unwrap();
        });
        let dir = tempfile::tempdir().unwrap();
        let mut host =
            LocalHost::new("fixture", "fixture:new").with_journal(dir.path().join("update.json"));
        host.binary = fixture(&dir);
        let value = serde_json::json!({
            "failure":failure, "exists":true, "image_version": env!("CARGO_PKG_VERSION"),
            "container": {"Mounts":[{"Destination":"/var/lib/guaca", "Type":"volume", "Name":format!("{}-data", host.spec.name)}], "Config":{"Image":"fixture:old", "Labels":{"bot.guaca.desktop":host.spec.name}},
                "State":{"Running":true},
                "NetworkSettings":{"Ports":{"8787/tcp":[{"HostIp":"127.0.0.1","HostPort":addr.port().to_string()}]}}}
        });
        std::fs::write(dir.path().join("docker-state.json"), value.to_string()).unwrap();
        (host, dir, task, format!("http://{addr}"))
    }
    /// The fake `docker` beside a fixture's own state.
    #[cfg(unix)]
    pub(crate) fn fixture(dir: &tempfile::TempDir) -> PathBuf {
        let binary = dir.path().join("docker");
        // A concurrent fork can inherit a freshly written executable's writer
        // until exec, causing ETXTBSY. Share the immutable script; __file__ still
        // names this symlink, so each fixture keeps its own state directory.
        std::os::unix::fs::symlink(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/docker-host.py"),
            &binary,
        )
        .unwrap();
        binary
    }
    #[cfg(unix)]
    fn calls(dir: &tempfile::TempDir) -> Vec<Vec<String>> {
        std::fs::read_to_string(dir.path().join("docker-calls.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    #[cfg(unix)]
    fn state(dir: &tempfile::TempDir) -> Value {
        serde_json::from_slice(&std::fs::read(dir.path().join("docker-state.json")).unwrap())
            .unwrap()
    }
    #[cfg(unix)]
    fn edit(dir: &tempfile::TempDir, change: impl FnOnce(&mut Value)) {
        let mut value = state(dir);
        change(&mut value);
        std::fs::write(dir.path().join("docker-state.json"), value.to_string()).unwrap();
    }
    #[test]
    fn a_refused_pull_is_not_blamed_on_the_connection() {
        let image = "ghcr.io/madebywelch/guaca/guacad:0.2.0";
        for refused in [
            "Docker could not finish: Error response from daemon: error from registry: denied",
            "Docker could not finish: Error response from daemon: manifest for ghcr.io/madebywelch/guaca/guacad:9.9.9 not found: manifest unknown",
            "Docker could not finish: Error response from daemon: Head \"https://ghcr.io/v2/madebywelch/guaca/guacad/manifests/0.2.0\": unauthorized",
        ] {
            let said = pull_failure(image, refused);
            assert!(said.contains("refused") && said.contains(image), "{refused}: {said}");
            assert!(!said.contains("connection"), "{refused}: {said}");
        }
        for offline in [
            "Docker could not finish: Error response from daemon: Get \"https://ghcr.io/v2/\": dial tcp: lookup ghcr.io: no such host",
            "Docker took too long to answer. Check Docker and try again.",
        ] {
            assert!(pull_failure(image, offline).contains("Check your connection"), "{offline}");
        }
    }

    #[test]
    fn every_stage_reads_back_as_itself_and_an_older_journal_still_reads() {
        for stage in Stage::ALL {
            let said = serde_json::to_string(&stage).unwrap();
            assert_eq!(serde_json::from_str::<Stage>(&said).unwrap(), stage);
        }
        let older: Operation = serde_json::from_value(serde_json::json!({
            "stage":"Verifying host", "backup":"saved", "previousImage":"old",
            "targetImage":"new", "error":null
        }))
        .unwrap();
        assert_eq!(older.stage, Stage::Verifying);
        assert!(older.target().is_none(), "an older journal names no version to verify");
        assert!(serde_json::from_str::<Stage>("\"Reticulating\"").is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn download_and_stop_failures_never_replace_the_container() {
        for mode in ["download", "stop"] {
            let (host, dir, task, origin) = simulated(mode, env!("CARGO_PKG_VERSION")).await;
            assert!(host.update(&host.bundled(), Some(&origin)).await.is_err());
            assert!(!calls(&dir).iter().any(|c| c[0] == "rm" || c[0] == "run"));
            assert_eq!(host.operation().unwrap().unwrap().stage, Stage::Canceled);
            task.abort();
        }
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn backup_failure_restarts_only_the_untouched_old_container() {
        for mode in ["backup", "backup-restart"] {
            let (host, dir, task, origin) = simulated(mode, env!("CARGO_PKG_VERSION")).await;
            let error = host.update(&host.bundled(), Some(&origin)).await.err().unwrap();
            assert!(
                error.contains(if mode == "backup" {
                    "previous host was restarted"
                } else {
                    "could not restart"
                }),
                "{error}"
            );
            let log = calls(&dir);
            assert!(log.iter().any(|c| c[0] == "start"));
            assert!(!log.iter().any(|c| c[0] == "rm"));
            task.abort();
        }
    }
    /// Volumes beside a fixture host: two of its own earlier copies, and
    /// three that only look like them.
    #[cfg(unix)]
    fn earlier_copies(dir: &tempfile::TempDir) {
        edit(dir, |state| {
            state["volumes"] = serde_json::json!({
                "fixture-host-backup-earlier": "fixture-host-data",
                "fixture-host-failed-earlier": "fixture-host-data",
                "fixture-host-data": "",
                "fixture-host-backups-notes": "",
                "other-host-backup-1": "other-host-data",
            });
        });
    }
    #[cfg(unix)]
    fn volumes(dir: &tempfile::TempDir) -> Vec<String> {
        let mut names: Vec<String> =
            state(dir)["volumes"].as_object().unwrap().keys().cloned().collect();
        names.sort();
        names
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn a_backup_that_will_not_go_does_not_stop_the_update() {
        let (host, dir, task, origin) = simulated("prune", env!("CARGO_PKG_VERSION")).await;
        earlier_copies(&dir);
        host.update(&host.bundled(), Some(&origin)).await.unwrap();
        assert_eq!(host.operation().unwrap().unwrap().stage, Stage::Updated);
        assert!(volumes(&dir).contains(&"fixture-host-backup-earlier".to_string()));
        task.abort();
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn a_backup_that_failed_removes_nothing() {
        let (host, dir, task, origin) = simulated("backup", env!("CARGO_PKG_VERSION")).await;
        earlier_copies(&dir);
        assert!(host.update(&host.bundled(), Some(&origin)).await.is_err());
        assert!(!calls(&dir).iter().any(|c| c[..2] == ["volume", "rm"]));
        assert!(volumes(&dir).contains(&"fixture-host-backup-earlier".to_string()));
        task.abort();
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn an_update_keeps_one_backup_the_one_it_took() {
        let (host, dir, task, origin) = simulated("", env!("CARGO_PKG_VERSION")).await;
        earlier_copies(&dir);
        host.update(&host.bundled(), Some(&origin)).await.unwrap();
        let taken = host.operation().unwrap().unwrap().backup.unwrap();
        let mut expected = vec![
            taken.clone(),
            "fixture-host-backups-notes".to_string(),
            "fixture-host-data".to_string(),
            "other-host-backup-1".to_string(),
        ];
        expected.sort();
        assert_eq!(volumes(&dir), expected, "only this host's earlier copies go");
        // Removed after the new one was taken and recorded, never before.
        let log = calls(&dir);
        let copied = log.iter().position(|c| c[0] == "run" && c.contains(&"cp".into())).unwrap();
        let first_removal = log.iter().position(|c| c[..2] == ["volume", "rm"]).unwrap();
        assert!(copied < first_removal);
        task.abort();
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn a_failed_replacement_is_restored_from_the_backup_on_the_previous_image() {
        for (mode, version) in [("create", env!("CARGO_PKG_VERSION")), ("", "99.0.0")] {
            let (host, dir, task, origin) = simulated(mode, version).await;
            let error = host.update(&host.bundled(), Some(&origin)).await.err().unwrap();
            assert!(error.contains("put the previous version back"), "{mode}: {error}");
            let op = host.operation().unwrap().unwrap();
            assert_eq!(op.stage, Stage::Restored, "{mode}: {error}");
            let backup = op.backup.clone().unwrap();
            let preserved = op.preserved.clone().unwrap();
            assert!(preserved.starts_with("fixture-host-failed-"), "{preserved}");
            let now = state(&dir);
            assert_eq!(now["container"]["Config"]["Image"], "fixture:old", "{mode}");
            assert_eq!(now["restored_from"], backup.as_str(), "{mode}");
            assert_eq!(now["volumes"][&preserved], "fixture-host-data", "{mode}");
            // The failed workspace was copied aside before the backup went over it.
            let log = calls(&dir);
            let aside = log.iter().position(|c| c.iter().any(|a| a.contains(&preserved))).unwrap();
            let restored = log.iter().position(|c| c.iter().any(|a| a == "sh")).unwrap();
            assert!(aside < restored, "{mode}");
            // A restored host starts normally afterward.
            host.start().await.unwrap();
            task.abort();
        }
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn a_restore_that_fails_leaves_recovery_state_and_refuses_to_start() {
        for (mode, version) in
            [("create-always", env!("CARGO_PKG_VERSION")), ("preserve", "99.0.0")]
        {
            let (host, dir, task, origin) = simulated(mode, version).await;
            let error = host.update(&host.bundled(), Some(&origin)).await.err().unwrap();
            assert!(error.contains("could not be restored"), "{mode}: {error}");
            let op = host.operation().unwrap().unwrap();
            assert_eq!(op.stage, Stage::RecoveryNeeded, "{mode}");
            assert!(error.contains(op.backup.as_deref().unwrap()), "{mode}: {error}");
            if mode == "preserve" {
                // Nothing may be deleted without a copy of it somewhere.
                assert!(!calls(&dir).iter().any(|c| c.iter().any(|a| a == "sh")));
            }
            let restart_error = host.start().await.err().unwrap();
            assert!(restart_error.contains("could not be undone"), "{restart_error}");
            task.abort();
        }
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn replacement_is_verified_and_bound_to_the_selected_host() {
        let (host, dir, task, origin) = simulated("", env!("CARGO_PKG_VERSION")).await;
        assert!(host.update(&host.bundled(), Some("https://another-host.example")).await.is_err());
        assert!(!calls(&dir).iter().any(|c| c[0] == "stop"));
        let updated = host.update(&host.bundled(), Some(&origin)).await.unwrap();
        assert_eq!(updated.origin, origin);
        assert_eq!(updated.token, "fixture-token");
        let op = host.operation().unwrap().unwrap();
        assert_eq!(op.stage, Stage::Updated);
        assert_eq!(op.port, Some(origin.rsplit(':').next().unwrap().parse().unwrap()));
        let log = calls(&dir);
        let backup = log.iter().position(|c| c[0] == "run" && c.contains(&"cp".into())).unwrap();
        let removed = log.iter().position(|c| c[0] == "rm").unwrap();
        assert!(backup < removed);
        let record =
            LocalHost::new("fixture", "fixture:new").with_journal(dir.path().join("update.json"));
        assert_eq!(record.operation().unwrap().unwrap().backup, op.backup);
        let again = host.update(&host.bundled(), Some(&origin)).await.err().unwrap();
        assert!(again.contains("already runs"), "{again}");
        task.abort();
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn an_image_from_another_release_is_refused_before_the_host_stops() {
        let (host, dir, task, origin) = simulated("", env!("CARGO_PKG_VERSION")).await;
        edit(&dir, |state| state["image_version"] = serde_json::json!("0.1.0"));
        let error = host.update(&host.bundled(), Some(&origin)).await.err().unwrap();
        assert!(error.contains("reports version 0.1.0"), "{error}");
        assert!(!calls(&dir).iter().any(|c| c[0] == "stop" || c[0] == "rm" || c[0] == "run"));
        assert_eq!(host.operation().unwrap().unwrap().stage, Stage::Canceled);
        task.abort();
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn a_newer_host_is_never_replaced_by_an_older_target() {
        let (host, dir, task, origin) = simulated("", env!("CARGO_PKG_VERSION")).await;
        edit(&dir, |state| {
            state["container"]["Config"]["Labels"]["org.opencontainers.image.version"] =
                serde_json::json!("99.0.0")
        });
        let error = host.update(&host.bundled(), Some(&origin)).await.err().unwrap();
        assert!(error.contains("newer than Guaca"), "{error}");
        assert!(!calls(&dir).iter().any(|c| c[0] == "pull" || c[0] == "stop"));
        let op = host.operation().unwrap().unwrap();
        assert_eq!(op.stage, Stage::Canceled);
        assert_eq!(op.error.as_deref(), Some(error.as_str()));
        task.abort();
    }

    /// An interrupted journal, as a manager killed at `stage` leaves it.
    #[cfg(unix)]
    fn interrupted(host: &LocalHost, stage: Stage, port: u16) -> Operation {
        let op = Operation {
            stage,
            backup: Some("fixture-host-backup-1".into()),
            previous_image: "fixture:old".into(),
            target_image: "fixture:new".into(),
            target_version: env!("CARGO_PKG_VERSION").into(),
            api_generation: crate::updates::protocol().generation,
            port: Some(port),
            preserved: None,
            error: None,
        };
        host.record(&op).unwrap();
        op
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn a_manager_killed_before_the_swap_keeps_the_previous_host() {
        for stage in [Stage::Downloading, Stage::Stopping, Stage::BackingUp] {
            let (host, dir, task, origin) = simulated("", env!("CARGO_PKG_VERSION")).await;
            edit(&dir, |state| state["container"]["State"]["Running"] = serde_json::json!(false));
            interrupted(&host, stage, origin.rsplit(':').next().unwrap().parse().unwrap());
            host.start().await.unwrap();
            let op = host.operation().unwrap().unwrap();
            assert_eq!(op.stage, Stage::Canceled, "{stage:?}");
            let log = calls(&dir);
            assert!(!log.iter().any(|c| c[0] == "rm" || c[0] == "run"), "{stage:?}");
            assert_eq!(state(&dir)["container"]["Config"]["Image"], "fixture:old");
            task.abort();
        }
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn a_manager_killed_after_the_swap_keeps_a_host_that_verifies() {
        let (host, dir, task, origin) = simulated("", env!("CARGO_PKG_VERSION")).await;
        edit(&dir, |state| {
            state["container"]["Config"]["Image"] = serde_json::json!("fixture:new")
        });
        interrupted(&host, Stage::Verifying, origin.rsplit(':').next().unwrap().parse().unwrap());
        host.start().await.unwrap();
        assert_eq!(host.operation().unwrap().unwrap().stage, Stage::Updated);
        assert!(!calls(&dir).iter().any(|c| c[0] == "rm" || c[0] == "run"));
        task.abort();
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn a_manager_killed_after_the_swap_restores_a_host_that_does_not() {
        for (stage, image) in [
            (Stage::Starting, None),
            (Stage::Verifying, Some("fixture:old")),
            (Stage::Restoring, Some("fixture:new")),
        ] {
            let (host, dir, task, origin) = simulated("", "99.0.0").await;
            edit(&dir, |state| match image {
                Some(image) => state["container"]["Config"]["Image"] = serde_json::json!(image),
                None => state["exists"] = serde_json::json!(false),
            });
            let port: u16 = origin.rsplit(':').next().unwrap().parse().unwrap();
            interrupted(&host, stage, port);
            let restored = host.start().await.unwrap();
            assert_eq!(restored.origin, origin, "{stage:?} keeps its port");
            let op = host.operation().unwrap().unwrap();
            assert_eq!(op.stage, Stage::Restored, "{stage:?}");
            let now = state(&dir);
            assert_eq!(now["container"]["Config"]["Image"], "fixture:old", "{stage:?}");
            assert_eq!(now["restored_from"], "fixture-host-backup-1", "{stage:?}");
            task.abort();
        }
    }

    #[test]
    fn an_unpinned_app_installs_the_host_of_its_own_release() {
        if option_env!("GUACA_BACKEND_IMAGE").is_none() {
            assert_eq!(
                IMAGE,
                format!("ghcr.io/madebywelch/guaca/guacad:{}", env!("CARGO_PKG_VERSION"))
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_reconfigured_workspace_is_never_backed_up_from_a_guessed_volume() {
        let (host, dir, task, origin) = simulated("", env!("CARGO_PKG_VERSION")).await;
        edit(&dir, |state| {
            state["container"]["Mounts"][0]["Name"] = serde_json::json!("manually-configured-data")
        });
        assert!(host
            .update(&host.bundled(), Some(&origin))
            .await
            .err()
            .unwrap()
            .contains("different workspace volume"));
        assert!(!calls(&dir).iter().any(|c| c[0] == "stop" || c[0] == "run"));
        task.abort();
    }

    #[test]
    fn unreadable_and_unrecoverable_journals_refuse_automatic_startup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update.json");
        let host = LocalHost::new("fixture", "fixture:new").with_journal(path.clone());
        std::fs::write(&path, "broken JSON").unwrap();
        assert!(host.stuck().is_err());
        host.record(&Operation {
            stage: Stage::RecoveryNeeded,
            backup: Some("saved-volume".into()),
            previous_image: "old".into(),
            target_image: "new".into(),
            target_version: String::new(),
            api_generation: 0,
            port: None,
            preserved: None,
            error: None,
        })
        .unwrap();
        assert!(host.stuck().unwrap_err().contains("saved-volume"));
        let first = host.process_lock().unwrap();
        assert!(host.process_lock().is_err());
        drop(first);
        assert!(host.process_lock().is_ok());
    }
    #[cfg(unix)]
    #[test]
    fn an_inherited_descriptor_does_not_keep_a_finished_operation_locked() {
        let dir = tempfile::tempdir().unwrap();
        let host =
            LocalHost::new("fixture", "fixture:new").with_journal(dir.path().join("update.json"));
        let operation = host.process_lock().unwrap().unwrap();
        // A fork inherits the same open file description until exec closes it.
        let inherited = operation.0.try_clone().unwrap();
        assert!(host.process_lock().is_err());
        drop(operation);
        let next = host.process_lock().expect("the finished operation must release its lock");
        drop(inherited);
        assert!(host.process_lock().is_err());
        drop(next);
        assert!(host.process_lock().is_ok());
    }

    #[test]
    fn an_older_target_cannot_downgrade_a_newer_host() {
        let value =
            serde_json::json!({"Config":{"Labels":{"org.opencontainers.image.version":"99.0.0"}}});
        assert!(newer_than(&value, env!("CARGO_PKG_VERSION")));
        assert!(!newer_than(&value, "99.0.0"));
        assert!(!newer_than(&serde_json::json!({}), env!("CARGO_PKG_VERSION")));
    }
    #[tokio::test]
    #[ignore = "requires Docker and GUACA_TEST_IMAGE; no model calls"]
    async fn docker_host_survives_client_and_container_restarts() {
        let image = std::env::var("GUACA_TEST_IMAGE").expect("GUACA_TEST_IMAGE");
        let id = format!("guaca-test-{}", uuid::Uuid::new_v4());
        let host = LocalHost::new(&id, &image);
        let first = host.start().await.unwrap();
        let second = host.start().await.unwrap();
        assert_eq!(first.origin, second.origin);
        assert_eq!(first.token, second.token);
        let http = reqwest::Client::new();
        let reply: Value = http.post(format!("{}/v1/call", first.origin)).bearer_auth(&first.token)
            .json(&serde_json::json!({"name":"create_group","args":{"draft":{"name":"Persistent test"}}}))
            .send().await.unwrap().json().await.unwrap();
        assert_eq!(reply["ok"]["name"], "Persistent test");
        let container = host.spec.name.clone();
        drop(host);
        docker(&["stop", &container], 45).await.unwrap();
        // A second tag of the same image is a different reference, which is
        // all an update compares, and it reports the same release.
        let next = format!("{id}:next");
        docker(&["tag", &image, &next], 30).await.unwrap();
        let journal = tempfile::tempdir().unwrap();
        let host = LocalHost::new(&id, &next).with_journal(journal.path().join("update.json"));
        let resumed = host.start().await.unwrap();
        assert_eq!(first.token, resumed.token);
        let reply: Value = http
            .post(format!("{}/v1/call", resumed.origin))
            .bearer_auth(&resumed.token)
            .json(&serde_json::json!({"name":"list_groups","args":{}}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(reply["ok"].as_array().unwrap().iter().any(|g| g["name"] == "Persistent test"));
        let updated = host.update(&host.bundled(), Some(&resumed.origin)).await.unwrap();
        assert_eq!(updated.origin, resumed.origin);
        assert_eq!(updated.token, resumed.token);
        let groups: Value = http
            .post(format!("{}/v1/call", updated.origin))
            .bearer_auth(&updated.token)
            .json(&serde_json::json!({"name":"list_groups","args":{}}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(groups["ok"].as_array().unwrap().iter().any(|g| g["name"] == "Persistent test"));
        // An update the new image cannot pass is undone with the workspace
        // intact. Its label claims a release its binary does not report, so
        // it passes the check before the swap and fails the one after.
        let wrong = format!("{id}:wrong");
        let mut build = std::process::Command::new(docker_binary())
            .args(["build", "--quiet", "--tag", &wrong, "-"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        {
            use std::io::Write;
            let mut stdin = build.stdin.take().unwrap();
            writeln!(stdin, "FROM {image}\nLABEL org.opencontainers.image.version=99.0.0").unwrap();
        }
        assert!(build.wait().unwrap().success());
        let target = Target { image: wrong.clone(), version: "99.0.0".into(), ..host.bundled() };
        let error = host.update(&target, Some(&updated.origin)).await.err().unwrap();
        assert!(error.contains("put the previous version back"), "{error}");
        let op = host.operation().unwrap().unwrap();
        assert_eq!(op.stage, Stage::Restored);
        let restored = host.start().await.unwrap();
        assert_eq!(restored.origin, updated.origin, "the port is kept");
        let running = host.running().await.unwrap().unwrap();
        assert_eq!(running.image, next, "the previous image runs again");
        let groups: Value = http
            .post(format!("{}/v1/call", restored.origin))
            .bearer_auth(&restored.token)
            .json(&serde_json::json!({"name":"list_groups","args":{}}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(
            groups["ok"].as_array().unwrap().iter().any(|g| g["name"] == "Persistent test"),
            "the restored workspace is the one from before the update"
        );
        let volumes = docker(
            &["volume", "ls", "--filter", &format!("name={container}-"), "--format", "{{.Name}}"],
            15,
        )
        .await
        .unwrap();
        let backups: Vec<&str> = volumes.lines().filter(|v| v.contains("-backup-")).collect();
        assert_eq!(backups.len(), 2, "one backup per update: {volumes}");
        let failed = volumes.lines().filter(|v| v.contains("-failed-")).count();
        assert_eq!(failed, 1, "the failed version's workspace is kept: {volumes}");
        for backup in &backups {
            let result = docker(
                &[
                    "run",
                    "--rm",
                    "--entrypoint",
                    "test",
                    "--mount",
                    &format!("type=volume,src={backup},dst=/saved,readonly"),
                    &image,
                    "-s",
                    "/saved/data/guac.db",
                ],
                30,
            )
            .await;
            assert!(result.is_ok(), "backup contains the database");
        }
        docker(&["rm", "-f", &container], 45).await.unwrap();
        for volume in volumes.lines() {
            docker(&["volume", "rm", volume], 30).await.unwrap();
        }
        for tag in ["next", "wrong"] {
            let _ = docker(&["rmi", &format!("{id}:{tag}")], 30).await;
        }
    }
    #[test]
    fn only_a_loopback_binding_is_accepted() {
        let value = serde_json::json!({"NetworkSettings":{"Ports":{"8787/tcp":[{"HostIp":"0.0.0.0","HostPort":"8787"}]}}});
        assert!(published_port(&value).is_err());
        let value = serde_json::json!({"NetworkSettings":{"Ports":{"8787/tcp":[{"HostIp":"127.0.0.1","HostPort":"51234"}]}}});
        assert_eq!(published_port(&value).unwrap(), 51234);
    }
    #[test]
    fn preview_and_installed_app_have_independent_hosts() {
        assert_ne!(
            LocalHost::new("com.madebywelch.guac", IMAGE).spec.name,
            LocalHost::new("com.madebywelch.guac.preview", IMAGE).spec.name
        );
    }
}
