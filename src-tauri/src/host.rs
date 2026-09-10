//! The desktop's local Docker host. Never starts an agent in this process.
//! Container names are scoped to the bundle ID; volumes survive app upgrades.
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::process::Command;
use tokio::sync::Mutex;

pub const IMAGE: &str = match option_env!("GUACA_BACKEND_IMAGE") {
    Some(image) => image,
    None => "ghcr.io/madebywelch/guaca/guacad:0.1.0",
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub origin: String,
    pub token: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Operation {
    pub stage: String,
    pub backup: Option<String>,
    pub previous_image: String,
    pub target_image: String,
    pub error: Option<String>,
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExistingHost {
    pub name: String,
    pub label: String,
    pub origin: String,
}

pub struct LocalHost {
    name: String,
    image: String,
    lock: Mutex<()>,
    journal: Option<PathBuf>,
    binary: PathBuf,
}

fn docker_binary() -> PathBuf {
    std::env::var_os("PATH")
        .and_then(|p| std::env::split_paths(&p).map(|p| p.join("docker")).find(|p| p.is_file()))
        .or_else(|| {
            let p =
                std::path::PathBuf::from("/Applications/Docker.app/Contents/Resources/bin/docker");
            p.is_file().then_some(p)
        })
        .unwrap_or_else(|| "docker".into())
}

async fn run_docker(binary: &Path, args: &[&str], seconds: u64) -> Result<String, String> {
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

    pub fn new(identifier: &str, image: &str) -> Self {
        let name: String =
            identifier.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
        Self {
            name: format!("{name}-host"),
            image: image.to_string(),
            lock: Mutex::new(()),
            journal: None,
            binary: docker_binary(),
        }
    }

    async fn inspect(&self) -> Result<Option<Value>, String> {
        // Listing first distinguishes an absent container from an unavailable daemon.
        let ids = self
            .docker(
                &[
                    "container",
                    "ls",
                    "-a",
                    "--filter",
                    &format!("name=^/{}$", self.name),
                    "--format",
                    "{{.ID}}",
                ],
                15,
            )
            .await?;
        if ids.is_empty() {
            return Ok(None);
        }
        let text = self.docker(&["container", "inspect", &self.name], 15).await?;
        let list: Vec<Value> = serde_json::from_str(&text)
            .map_err(|_| "Docker returned an unreadable container.".to_string())?;
        let value = list.into_iter().next().ok_or("Docker returned no container.")?;
        if value["Config"]["Labels"]["bot.guaca.desktop"] != self.name {
            return Err(
                "A different container is using Guaca's name. It has been left untouched.".into()
            );
        }
        Ok(Some(value))
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

    pub fn is_updating(&self) -> bool {
        self.lock.try_lock().is_err()
    }

    fn operation(&self) -> Result<Option<Operation>, String> {
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
        tracing::info!(container = %self.name, stage = %operation.stage, backup = ?operation.backup, "local host update");
        Ok(())
    }

    fn recovery(&self) -> Result<(), String> {
        if let Some(op) = self.operation()? {
            if matches!(
                op.stage.as_str(),
                "Starting updated host" | "Verifying host" | "Recovery needed"
            ) {
                return Err(format!("The previous host update needs recovery. Backup: {}. Review the update instructions before starting or replacing this host.", op.backup.as_deref().unwrap_or("see Docker volumes")));
            }
        }
        Ok(())
    }

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
                status.update_available = value["Config"]["Image"].as_str() != Some(&self.image)
                    && !newer_than_app(&value)
                    && self.recovery().is_ok();
                if let Err(error) = self.recovery() {
                    status.message = error;
                }
                if newer_than_app(&value) {
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

    fn process_lock(&self) -> Result<Option<std::fs::File>, String> {
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
        Ok(Some(file))
    }

    pub async fn start(&self) -> Result<Connection, String> {
        let _lock = self.lock.lock().await;
        let _process_lock = self.process_lock()?;
        self.recovery()?;
        self.start_unlocked(None).await
    }

    async fn image_ready(&self) -> Result<(), String> {
        if self.docker(&["image", "inspect", &self.image], 15).await.is_err() {
            self.docker(&["pull", &self.image], 900).await.map_err(|_| "The Guaca host could not be downloaded. Check your connection and try again. Source installs can build it with scripts/install.sh.".to_string())?;
        }
        Ok(())
    }

    /// Update is explicit: download before interrupting work, then preserve a
    /// complete stopped-volume backup before the new binary can migrate it.
    pub async fn update(&self, expected_origin: Option<&str>) -> Result<Connection, String> {
        let _lock = self
            .lock
            .try_lock()
            .map_err(|_| "A host operation is already running. Wait for it to finish.")?;
        let _process_lock = self.process_lock()?;
        self.recovery()?;
        self.available().await?;
        let old = self
            .inspect()
            .await?
            .ok_or("The connected local host no longer exists. Reconnect before updating.")?;
        let expected_volume = format!("{}-data", self.name);
        let correct_volume = old["Mounts"].as_array().is_some_and(|mounts| {
            mounts.iter().any(|mount| {
                mount["Destination"] == "/var/lib/guaca"
                    && mount["Type"] == "volume"
                    && mount["Name"] == expected_volume
            })
        });
        if !correct_volume {
            return Err("This container uses a different workspace volume than the one Guaca manages. It was left untouched. Update it using the self-hosted instructions.".into());
        }
        let port = published_port(&old)?;
        if expected_origin.is_some_and(|origin| origin != format!("http://127.0.0.1:{port}")) {
            return Err("The selected workspace is not the local host this app manages. Reconnect before updating.".into());
        }
        if newer_than_app(&old) {
            return Err("This host is newer than this desktop app. Update Guaca first.".into());
        }
        let mut op = Operation {
            stage: "Downloading update".into(),
            backup: None,
            previous_image: old["Config"]["Image"].as_str().unwrap_or_default().into(),
            target_image: self.image.clone(),
            error: None,
        };
        self.record(&op)?;
        let result = self.replace(&mut op, port).await;
        if let Err(error) = &result {
            op.stage = if matches!(op.stage.as_str(), "Starting updated host" | "Verifying host") {
                "Recovery needed"
            } else {
                "Update canceled"
            }
            .into();
            op.error = Some(error.clone());
            if let Err(record_error) = self.record(&op) {
                tracing::error!(%record_error, %error, "could not record update failure");
            }
        }
        result
    }

    async fn replace(&self, op: &mut Operation, port: u16) -> Result<Connection, String> {
        self.image_ready().await?;
        let raw = self.docker(&["image", "inspect", &self.image], 15).await?;
        let image: Vec<Value> =
            serde_json::from_str(&raw).map_err(|_| "Docker returned unreadable image metadata.")?;
        let revision = image
            .first()
            .and_then(|v| v["Config"]["Labels"]["org.opencontainers.image.revision"].as_str())
            .unwrap_or_default()
            .to_string();
        op.stage = "Stopping host".into();
        self.record(op)?;
        self.docker(&["stop", &self.name], 60).await?;
        let backup = format!("{}-backup-{}", self.name, uuid::Uuid::new_v4());
        let volume = format!("{}-data", self.name);
        op.backup = Some(backup.clone());
        op.stage = "Backing up workspace".into();
        // If journaling fails after stopping, restart the old host as with a
        // failed copy. The target has not mounted the live workspace yet.
        let saved = match self.record(op) {
            Err(e) => Err(e),
            Ok(()) => {
                self.docker(
                    &[
                        "run",
                        "--rm",
                        "--user",
                        "0",
                        "--entrypoint",
                        "cp",
                        "--mount",
                        &format!("type=volume,src={volume},dst=/source,readonly"),
                        "--mount",
                        &format!("type=volume,src={backup},dst=/backup"),
                        &self.image,
                        "-a",
                        "/source/.",
                        "/backup/",
                    ],
                    600,
                )
                .await
            }
        };
        if saved.is_err() {
            return match self.docker(&["start", &self.name], 60).await {
                Ok(_) => Err("The host backup could not be completed. The update was canceled and the previous host was restarted.".into()),
                Err(_) => Err("The backup failed and Docker could not restart the previous host. Your data is preserved. Check Docker before trying again.".into()),
            };
        }
        // Persist the recovery point before any destructive action. An app
        // killed after this write must not automatically start another image.
        op.stage = "Starting updated host".into();
        self.record(op)?;
        self.docker(&["rm", &self.name], 30).await?;
        let connection = self.start_unlocked(Some(port)).await?;
        op.stage = "Verifying host".into();
        self.record(op)?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| e.to_string())?;
        let health: Value = http
            .get(format!("{}/health", connection.origin))
            .send()
            .await
            .map_err(|e| format!("The updated host did not answer: {e}. Backup: {backup}"))?
            .json()
            .await
            .map_err(|_| "The updated host returned unreadable health information.")?;
        if (!revision.is_empty() && health["build"].as_str() != Some(&revision))
            || health["version"] != env!("CARGO_PKG_VERSION")
            || health["apiGeneration"] != crate::updates::protocol().generation
        {
            return Err(format!(
                "The updated host reported an unexpected version or API. Backup: {backup}"
            ));
        }
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
            return Err(format!(
                "The updated workspace did not accept its access key. Backup: {backup}"
            ));
        }
        op.stage = "Host updated".into();
        self.record(op)?;
        Ok(connection)
    }

    async fn start_unlocked(&self, port: Option<u16>) -> Result<Connection, String> {
        self.available().await?;
        if self.inspect().await?.is_none() {
            self.image_ready().await?;
            let binding = port
                .map(|p| format!("127.0.0.1:{p}:8787"))
                .unwrap_or_else(|| "127.0.0.1::8787".into());
            let volume = format!("{}-data", self.name);
            self.docker(
                &[
                    "run",
                    "--detach",
                    "--name",
                    &self.name,
                    "--label",
                    &format!("bot.guaca.desktop={}", self.name),
                    "--init",
                    "--restart",
                    "unless-stopped",
                    "--stop-timeout",
                    "30",
                    "--publish",
                    &binding,
                    "--mount",
                    &format!("type=volume,src={volume},dst=/var/lib/guaca"),
                    "--add-host",
                    "host.docker.internal:host-gateway",
                    &self.image,
                ],
                60,
            )
            .await?;
        } else {
            self.docker(&["start", &self.name], 60).await?;
        }
        let value = self.inspect().await?.ok_or("The local host disappeared while starting.")?;
        let port = published_port(&value)?;
        let origin = format!("http://127.0.0.1:{port}");
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|e| e.to_string())?;
        for _ in 0..60 {
            if let Ok(response) = http.get(format!("{origin}/health")).send().await {
                if response.status().is_success() {
                    let health: Value = response.json().await.unwrap_or_default();
                    if health["service"] == "guacad" {
                        let token = self
                            .docker(&["exec", &self.name, "cat", "/var/lib/guaca/config/token"], 10)
                            .await?;
                        if token.is_empty() {
                            return Err(
                                "The local host has not created its access key. Try again.".into(),
                            );
                        }
                        return Ok(Connection { origin, token });
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        Err("The local host started but is not ready to connect. Check Docker's Guaca container for details, then try again. Your data is preserved.".into())
    }
}

fn newer_than_app(value: &Value) -> bool {
    let current = value["Config"]["Labels"]["org.opencontainers.image.version"]
        .as_str()
        .and_then(|s| semver::Version::parse(s).ok());
    current.is_some_and(|v| v > semver::Version::parse(env!("CARGO_PKG_VERSION")).unwrap())
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

#[cfg(test)]
mod tests {
    use super::*;
    async fn docker(args: &[&str], seconds: u64) -> Result<String, String> {
        run_docker(&docker_binary(), args, seconds).await
    }
    #[cfg(unix)]
    async fn simulated(
        failure: &str,
        version: &str,
    ) -> (LocalHost, tempfile::TempDir, tokio::task::JoinHandle<()>, String) {
        use std::os::unix::fs::PermissionsExt;
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
        host.binary = dir.path().join("docker");
        std::fs::write(&host.binary, include_str!("../tests/fixtures/docker-host.py")).unwrap();
        std::fs::set_permissions(&host.binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        let value = serde_json::json!({
            "failure":failure, "exists":true,
            "container": {"Mounts":[{"Destination":"/var/lib/guaca", "Type":"volume", "Name":format!("{}-data", host.name)}], "Config":{"Image":"fixture:old", "Labels":{"bot.guaca.desktop":host.name}},
                "State":{"Running":true},
                "NetworkSettings":{"Ports":{"8787/tcp":[{"HostIp":"127.0.0.1","HostPort":addr.port().to_string()}]}}}
        });
        std::fs::write(dir.path().join("docker-state.json"), value.to_string()).unwrap();
        (host, dir, task, format!("http://{addr}"))
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
    #[tokio::test]
    async fn download_and_stop_failures_never_replace_the_container() {
        for mode in ["download", "stop"] {
            let (host, dir, task, origin) = simulated(mode, env!("CARGO_PKG_VERSION")).await;
            assert!(host.update(Some(&origin)).await.is_err());
            assert!(!calls(&dir).iter().any(|c| c[0] == "rm" || c[0] == "run"));
            assert_eq!(host.operation().unwrap().unwrap().stage, "Update canceled");
            task.abort();
        }
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn backup_failure_restarts_only_the_untouched_old_container() {
        for mode in ["backup", "backup-restart"] {
            let (host, dir, task, origin) = simulated(mode, env!("CARGO_PKG_VERSION")).await;
            let error = host.update(Some(&origin)).await.err().unwrap();
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
    #[cfg(unix)]
    #[tokio::test]
    async fn failed_replacement_and_wrong_version_leave_recovery_state() {
        for (mode, version) in [("create", env!("CARGO_PKG_VERSION")), ("", "99.0.0")] {
            let (host, dir, task, origin) = simulated(mode, version).await;
            assert!(host.update(Some(&origin)).await.is_err());
            let op = host.operation().unwrap().unwrap();
            assert_eq!(op.stage, "Recovery needed");
            assert!(op.backup.is_some());
            assert!(host.start().await.err().unwrap().contains("needs recovery"));
            assert!(!calls(&dir).iter().any(|c| c[0] == "start"));
            task.abort();
        }
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn replacement_is_verified_and_bound_to_the_selected_host() {
        let (host, dir, task, origin) = simulated("", env!("CARGO_PKG_VERSION")).await;
        assert!(host.update(Some("https://another-host.example")).await.is_err());
        assert!(!calls(&dir).iter().any(|c| c[0] == "stop"));
        let updated = host.update(Some(&origin)).await.unwrap();
        assert_eq!(updated.origin, origin);
        assert_eq!(updated.token, "fixture-token");
        let op = host.operation().unwrap().unwrap();
        assert_eq!(op.stage, "Host updated");
        let log = calls(&dir);
        let backup = log.iter().position(|c| c[0] == "run" && c.contains(&"cp".into())).unwrap();
        let removed = log.iter().position(|c| c[0] == "rm").unwrap();
        assert!(backup < removed);
        let record =
            LocalHost::new("fixture", "fixture:new").with_journal(dir.path().join("update.json"));
        assert_eq!(record.operation().unwrap().unwrap().backup, op.backup);
        task.abort();
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn a_reconfigured_workspace_is_never_backed_up_from_a_guessed_volume() {
        let (host, dir, task, origin) = simulated("", env!("CARGO_PKG_VERSION")).await;
        let path = dir.path().join("docker-state.json");
        let mut state: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        state["container"]["Mounts"][0]["Name"] = serde_json::json!("manually-configured-data");
        std::fs::write(path, state.to_string()).unwrap();
        assert!(host
            .update(Some(&origin))
            .await
            .err()
            .unwrap()
            .contains("different workspace volume"));
        assert!(!calls(&dir).iter().any(|c| c[0] == "stop" || c[0] == "run"));
        task.abort();
    }

    #[test]
    fn unreadable_and_interrupted_journals_refuse_automatic_startup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update.json");
        let host = LocalHost::new("fixture", "fixture:new").with_journal(path.clone());
        std::fs::write(&path, "broken JSON").unwrap();
        assert!(host.recovery().is_err());
        for stage in ["Starting updated host", "Verifying host", "Recovery needed"] {
            host.record(&Operation {
                stage: stage.into(),
                backup: Some("saved-volume".into()),
                previous_image: "old".into(),
                target_image: "new".into(),
                error: None,
            })
            .unwrap();
            assert!(host.recovery().unwrap_err().contains("saved-volume"));
        }
        let first = host.process_lock().unwrap();
        assert!(host.process_lock().is_err());
        drop(first);
        assert!(host.process_lock().is_ok());
    }
    #[test]
    fn an_older_app_cannot_downgrade_a_newer_host() {
        let value =
            serde_json::json!({"Config":{"Labels":{"org.opencontainers.image.version":"99.0.0"}}});
        assert!(newer_than_app(&value));
        assert!(!newer_than_app(&serde_json::json!({})));
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
        let container = host.name.clone();
        drop(host);
        docker(&["stop", &container], 45).await.unwrap();
        let host = LocalHost::new(&id, &image);
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
        let updated = host.update(Some(&resumed.origin)).await.unwrap();
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
        let backups = docker(
            &[
                "volume",
                "ls",
                "--filter",
                &format!("name={container}-backup-"),
                "--format",
                "{{.Name}}",
            ],
            15,
        )
        .await
        .unwrap();
        assert_eq!(backups.lines().count(), 1);
        for backup in backups.lines() {
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
            docker(&["volume", "rm", backup], 30).await.unwrap();
        }
        docker(&["rm", "-f", &container], 45).await.unwrap();
        docker(&["volume", "rm", &format!("{container}-data")], 30).await.unwrap();
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
            LocalHost::new("com.madebywelch.guac", IMAGE).name,
            LocalHost::new("com.madebywelch.guac.preview", IMAGE).name
        );
    }
}
