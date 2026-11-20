use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Engine {
    pub context: String,
    pub endpoint: String,
    pub id: String,
}

impl Engine {
    pub fn kind(&self) -> Kind {
        Kind::of(&self.context, &self.endpoint)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Named {
    pub context: String,
    pub label: String,
    pub action: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Colima(String),
    DockerDesktop,
    OrbStack,
    RancherDesktop,
    Other(String),
}

impl Kind {
    pub fn of(context: &str, endpoint: &str) -> Kind {
        let path = endpoint.strip_prefix("unix://").unwrap_or(endpoint);
        let parts: Vec<&str> = path.split('/').collect();
        if let Some(at) = parts.iter().position(|part| *part == ".colima") {
            let profile = parts.get(at + 2).and(parts.get(at + 1)).copied().unwrap_or("default");
            return Kind::Colima(profile.to_string());
        }
        if parts.contains(&".orbstack") || context == "orbstack" {
            return Kind::OrbStack;
        }
        if parts.contains(&".rd") || context == "rancher-desktop" {
            return Kind::RancherDesktop;
        }
        if context == "desktop-linux"
            || path.contains("/.docker/run/")
            || path.contains("/com.docker.docker/")
        {
            return Kind::DockerDesktop;
        }
        if let Some(profile) = context.strip_prefix("colima") {
            let profile = profile.strip_prefix('-').filter(|p| !p.is_empty()).unwrap_or("default");
            return Kind::Colima(profile.to_string());
        }
        Kind::Other(context.to_string())
    }

    pub fn label(&self) -> String {
        match self {
            Kind::Colima(profile) if profile == "default" => "Colima".into(),
            Kind::Colima(profile) => format!("Colima ({profile})"),
            Kind::DockerDesktop => "Docker Desktop".into(),
            Kind::OrbStack => "OrbStack".into(),
            Kind::RancherDesktop => "Rancher Desktop".into(),
            Kind::Other(context) => format!("Docker context \"{context}\""),
        }
    }

    pub fn action(&self) -> Option<String> {
        match self {
            Kind::Colima(_) => Some(format!("Start {}", self.label())),
            Kind::Other(_) => None,
            _ => Some(format!("Open {}", self.label())),
        }
    }

    pub fn named(&self, context: &str) -> Named {
        Named { context: context.into(), label: self.label(), action: self.action() }
    }

    pub async fn open(&self) -> Result<(), String> {
        match self {
            Kind::Colima(profile) => {
                if !profile.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                    return Err("Guaca could not tell which Colima profile to start.".into());
                }
                let mut child = tokio::process::Command::new(colima())
                    .args(["start", "--profile", profile])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .map_err(|_| {
                        "Guaca could not find Colima on this Mac, so it cannot start it."
                    })?;
                tokio::spawn(async move {
                    match child.wait().await {
                        Ok(status) if status.success() => {}
                        Ok(status) => tracing::warn!(%status, "colima start did not succeed"),
                        Err(error) => tracing::warn!(%error, "colima start could not be awaited"),
                    }
                });
                Ok(())
            }
            Kind::Other(_) => {
                Err(format!("Start {} yourself, then Guaca reconnects.", self.label()))
            }
            application => {
                let name = match application {
                    Kind::DockerDesktop => "Docker",
                    Kind::OrbStack => "OrbStack",
                    _ => "Rancher Desktop",
                };
                let opened = tokio::process::Command::new("/usr/bin/open")
                    .args(["-a", name])
                    .status()
                    .await
                    .is_ok_and(|status| status.success());
                if opened {
                    Ok(())
                } else {
                    Err(format!(
                        "Open {} from your applications, then Guaca reconnects.",
                        self.label()
                    ))
                }
            }
        }
    }
}

fn colima() -> std::path::PathBuf {
    std::env::var_os("PATH")
        .and_then(|p| std::env::split_paths(&p).map(|p| p.join("colima")).find(|p| p.is_file()))
        .or_else(|| {
            ["/opt/homebrew/bin/colima", "/usr/local/bin/colima"]
                .into_iter()
                .map(std::path::PathBuf::from)
                .find(|p| p.is_file())
        })
        .unwrap_or_else(|| "colima".into())
}

pub fn canonical(endpoint: &str) -> String {
    match endpoint.strip_prefix("unix://") {
        Some(path) => std::fs::canonicalize(path)
            .map(|real| format!("unix://{}", real.display()))
            .unwrap_or_else(|_| endpoint.to_string()),
        None => endpoint.to_string(),
    }
}

pub fn read(path: &Path) -> Option<Engine> {
    let bytes = std::fs::read(path).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(engine) => Some(engine),
        Err(error) => {
            tracing::warn!(%error, "the record of where the host runs is unreadable; looking again");
            None
        }
    }
}

pub fn write(path: &Path, engine: &Engine) -> Result<(), String> {
    let write = || -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp = path.with_extension("tmp");
        std::fs::write(&temp, serde_json::to_vec(engine)?)?;
        std::fs::rename(&temp, path)
    };
    write().map_err(|error| format!("Guaca could not record where its host runs: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_engine_is_named_by_where_its_socket_is() {
        let cases = [
            (
                "colima",
                "unix:///Users/r/.colima/default/docker.sock",
                "Colima",
                Some("Start Colima"),
            ),
            (
                "colima-work",
                "unix:///Users/r/.colima/work/docker.sock",
                "Colima (work)",
                Some("Start Colima (work)"),
            ),
            (
                "desktop-linux",
                "unix:///Users/r/.docker/run/docker.sock",
                "Docker Desktop",
                Some("Open Docker Desktop"),
            ),
            (
                "default",
                "unix:///Users/r/.docker/run/docker.sock",
                "Docker Desktop",
                Some("Open Docker Desktop"),
            ),
            (
                "orbstack",
                "unix:///Users/r/.orbstack/run/docker.sock",
                "OrbStack",
                Some("Open OrbStack"),
            ),
            (
                "rancher-desktop",
                "unix:///Users/r/.rd/docker.sock",
                "Rancher Desktop",
                Some("Open Rancher Desktop"),
            ),
            ("lab", "unix:///var/run/lab.sock", "Docker context \"lab\"", None),
        ];
        for (context, endpoint, label, action) in cases {
            let kind = Kind::of(context, endpoint);
            assert_eq!(kind.label(), label, "{context} {endpoint}");
            assert_eq!(kind.action().as_deref(), action, "{context} {endpoint}");
        }
    }

    #[test]
    fn a_record_reads_back_and_an_unreadable_one_is_no_record() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("host-engine.json");
        assert_eq!(read(&path), None);
        let engine = Engine {
            context: "colima".into(),
            endpoint: "unix:///Users/r/.colima/default/docker.sock".into(),
            id: "ENGINE".into(),
        };
        write(&path, &engine).unwrap();
        assert_eq!(read(&path), Some(engine));
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(read(&path), None);
    }
}
