//! The desktop's local Docker host. Never starts an agent in this process.
//! Container names are scoped to the bundle ID; volumes survive app upgrades.
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub state: &'static str,
    pub message: String,
    pub update_available: bool,
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
}

async fn docker(args: &[&str], seconds: u64) -> Result<String, String> {
    let binary = std::env::var_os("PATH")
        .and_then(|p| std::env::split_paths(&p).map(|p| p.join("docker")).find(|p| p.is_file()))
        .or_else(|| {
            let p =
                std::path::PathBuf::from("/Applications/Docker.app/Contents/Resources/bin/docker");
            p.is_file().then_some(p)
        })
        .unwrap_or_else(|| "docker".into());
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
    pub fn new(identifier: &str, image: &str) -> Self {
        let name: String =
            identifier.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
        Self { name: format!("{name}-host"), image: image.to_string(), lock: Mutex::new(()) }
    }

    async fn inspect(&self) -> Result<Option<Value>, String> {
        // Listing first distinguishes an absent container from an unavailable daemon.
        let ids = docker(
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
        let text = docker(&["container", "inspect", &self.name], 15).await?;
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
        let endpoint =
            docker(&["context", "inspect", "--format", "{{.Endpoints.docker.Host}}"], 10).await?;
        let endpoint = if std::env::var("DOCKER_CONTEXT").is_ok_and(|s| !s.is_empty()) {
            endpoint
        } else {
            std::env::var("DOCKER_HOST").unwrap_or(endpoint)
        };
        if !endpoint.starts_with("unix://") && !endpoint.starts_with("npipe://") {
            return Err("Docker is connected to another computer. Select a local Docker context for On this Mac.".into());
        }
        let os = docker(&["info", "--format", "{{.OSType}}"], 15).await
            .map_err(|_| "Docker is installed but is not ready. Open Docker Desktop, wait for it to start, then check again.".to_string())?;
        if os != "linux" {
            return Err("Guaca needs Docker's Linux containers. Switch Docker to Linux containers and check again.".into());
        }
        Ok(())
    }

    pub async fn existing(&self) -> Result<Vec<ExistingHost>, String> {
        self.available().await?;
        let names = docker(
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
            let raw = docker(&["container", "inspect", name], 15).await?;
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
        let token = docker(&["exec", &host.name, "cat", "/var/lib/guaca/config/token"], 15).await?;
        if token.is_empty() {
            return Err("This host has no saved access key. Connect with its address and key under Remote host.".into());
        }
        Ok(Connection { origin: host.origin, token })
    }

    pub async fn status(&self) -> Status {
        if let Err(message) = self.available().await {
            let state = if message.contains("not installed") { "missing" } else { "unavailable" };
            return Status { state, message, update_available: false };
        }
        match self.inspect().await {
            Ok(Some(value)) => Status {
                state: if value["State"]["Running"] == true { "running" } else { "stopped" },
                message: if value["State"]["Running"] == true {
                    "Docker is running. Your local host is available."
                } else {
                    "Docker is ready. Your local host is stopped."
                }
                .into(),
                update_available: value["Config"]["Image"].as_str() != Some(&self.image),
            },
            Ok(None) => Status {
                state: "ready",
                message: "Docker is ready. Guaca can set up your local host.".into(),
                update_available: false,
            },
            Err(message) => Status { state: "unavailable", message, update_available: false },
        }
    }

    pub async fn start(&self) -> Result<Connection, String> {
        let _lock = self.lock.lock().await;
        self.start_unlocked(None).await
    }

    async fn image_ready(&self) -> Result<(), String> {
        if docker(&["image", "inspect", &self.image], 15).await.is_err() {
            docker(&["pull", &self.image], 900).await.map_err(|_| "The Guaca host could not be downloaded. Check your connection and try again. Source installs can build it with scripts/install.sh.".to_string())?;
        }
        Ok(())
    }

    /// Update is explicit: download before interrupting work, then preserve a
    /// complete stopped-volume backup before the new binary can migrate it.
    pub async fn update(&self) -> Result<Connection, String> {
        let _lock = self.lock.lock().await;
        self.available().await?;
        let Some(old) = self.inspect().await? else {
            return self.start_unlocked(None).await;
        };
        self.image_ready().await?;
        let port = published_port(&old).ok();
        docker(&["stop", &self.name], 60).await?;
        let backup = format!("{}-backup-{}", self.name, uuid::Uuid::new_v4());
        let volume = format!("{}-data", self.name);
        let saved = docker(
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
        .await;
        if saved.is_err() {
            // The new image has not seen the live volume, so this rollback is safe.
            return match docker(&["start", &self.name], 60).await {
                Ok(_) => Err("The host backup could not be completed. The update was canceled and the previous host was restarted.".into()),
                Err(_) => Err("The host backup could not be completed. The update was canceled, but Docker could not restart the previous host. Your data is preserved; check Docker and try again.".into()),
            };
        }
        tracing::info!(container = %self.name, %backup, image = %self.image, "updating local host after volume backup");
        docker(&["rm", &self.name], 30).await?;
        self.start_unlocked(port).await.map_err(|error| {
            format!(
                "The updated host could not start. Your backup is Docker volume {backup}. {error}"
            )
        })
    }

    async fn start_unlocked(&self, port: Option<u16>) -> Result<Connection, String> {
        self.available().await?;
        if self.inspect().await?.is_none() {
            self.image_ready().await?;
            let binding = port
                .map(|p| format!("127.0.0.1:{p}:8787"))
                .unwrap_or_else(|| "127.0.0.1::8787".into());
            let volume = format!("{}-data", self.name);
            docker(
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
            docker(&["start", &self.name], 60).await?;
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
                        let token =
                            docker(&["exec", &self.name, "cat", "/var/lib/guaca/config/token"], 10)
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
        let updated = host.update().await.unwrap();
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
