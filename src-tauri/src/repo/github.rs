//! A workspace talks to its configured credential broker. The App's private
//! key is never mounted into the runtime or its coding jobs.
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::RepoError;

const SCRIPT: &str = include_str!("../../../deploy/github/github_app.py");

fn failed(message: &str) -> RepoError {
    RepoError::Connection(message.into())
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Connection {
    url: String,
    token_file: PathBuf,
    repository: String,
}

pub fn repository(remote: &str) -> Result<String, RepoError> {
    let url = super::auth::https_remote(remote)?;
    if url.scheme() != "https" || url.host_str() != Some("github.com") || url.port().is_some() {
        return Err(failed("GitHub App access needs an HTTPS github.com repository URL"));
    }
    let path = url.path().trim_start_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|s| {
            s.is_empty()
                || *s == "."
                || *s == ".."
                || !s.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
        })
    {
        return Err(failed("Use https://github.com/owner/repository for GitHub App access"));
    }
    Ok(path.to_ascii_lowercase())
}

pub fn configured() -> bool {
    std::env::var_os("GUACA_GITHUB_BROKER").is_some()
        && std::env::var_os("GUACA_GITHUB_BROKER_TOKEN_FILE").is_some()
}

pub fn file(credential: &Path) -> PathBuf {
    credential.with_extension("github.json")
}

fn quoted(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

pub fn helper(file: &Path) -> String {
    let script = file.parent().unwrap().join("github-helper.py");
    format!("!python3 {} credential {}", quoted(&script), quoted(file))
}

async fn save(file: &Path, bytes: &[u8], executable: bool) -> Result<(), RepoError> {
    use tokio::io::AsyncWriteExt;
    let parent = file.parent().ok_or_else(|| failed("Missing credential directory"))?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|_| failed("Could not create credential directory"))?;
    let pending = parent.join(format!(".{}", uuid::Uuid::new_v4()));
    let result = async {
        let mut options = tokio::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        options.mode(if executable { 0o700 } else { 0o600 });
        let mut output = options.open(&pending).await?;
        output.write_all(bytes).await?;
        output.sync_all().await?;
        drop(output);
        tokio::fs::rename(&pending, file).await
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(pending).await;
        return Err(failed("Could not save GitHub App connection"));
    }
    Ok(())
}

/// Validate access before replacing any existing Git credential configuration.
pub async fn prepare(file: &Path, remote: &str) -> Result<(), RepoError> {
    let connection = Connection {
        url: std::env::var("GUACA_GITHUB_BROKER").map_err(|_| failed("Configure a GitHub App credential service on this backend first (GUACA_GITHUB_BROKER)"))?,
        token_file: std::env::var_os("GUACA_GITHUB_BROKER_TOKEN_FILE").map(PathBuf::from).ok_or_else(|| failed("Configure GUACA_GITHUB_BROKER_TOKEN_FILE on this backend"))?,
        repository: repository(remote)?,
    };
    let mut url = reqwest::Url::parse(&connection.url)
        .map_err(|_| failed("Invalid GitHub credential service URL"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(failed("Invalid GitHub credential service URL"));
    }
    url.set_path(&format!("{}/v1/token", url.path().trim_end_matches('/')));
    let token = tokio::fs::read_to_string(&connection.token_file)
        .await
        .map_err(|_| failed("Could not read credential service authentication"))?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|_| failed("Could not start GitHub credential check"))?;
    let response = client
        .post(url)
        .bearer_auth(token.trim())
        .json(&serde_json::json!({"repository":connection.repository}))
        .send()
        .await
        .map_err(|_| failed("Could not reach GitHub credential service"))?;
    if !response.status().is_success() {
        return Err(failed("GitHub App access was refused. Check the broker's repository allowlist, installation, and Contents permission"));
    }
    let result: serde_json::Value =
        response.json().await.map_err(|_| failed("Invalid GitHub credential service response"))?;
    if result["token"].as_str().is_none_or(str::is_empty) {
        return Err(failed("GitHub credential service returned no installation token"));
    }
    // Tokens stay in memory. The connection only contains non-key configuration.
    let parent = file.parent().ok_or_else(|| failed("Missing credential directory"))?;
    save(&parent.join("github-helper.py"), SCRIPT.as_bytes(), false).await?;
    let wrapper =
        format!("#!/bin/sh\nexec python3 {} gh \"$@\"\n", quoted(&parent.join("github-helper.py")));
    save(&parent.join("github-bin/gh"), wrapper.as_bytes(), true).await?;
    save(
        file,
        &serde_json::to_vec(&connection).map_err(|_| failed("Invalid GitHub connection"))?,
        false,
    )
    .await
}

async fn config(path: &str, args: &[&str]) -> Result<(), RepoError> {
    let status = tokio::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .arg("config")
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|_| failed("Could not configure repository GitHub access"))?;
    if !status.status.success() {
        return Err(failed("Could not configure repository GitHub access"));
    }
    Ok(())
}

pub async fn attach(path: &str, file: &Path) -> Result<(), RepoError> {
    config(path, &["--local", "--replace-all", "credential.helper", ""]).await?;
    config(path, &["--local", "--add", "credential.helper", &helper(file)]).await?;
    config(path, &["--local", "credential.useHttpPath", "true"]).await?;
    config(path, &["--local", "guaca.githubConnection", &file.to_string_lossy()]).await
}

pub async fn attached(path: &str) -> Option<PathBuf> {
    let result = tokio::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["config", "--get", "guaca.githubConnection"])
        .output()
        .await
        .ok()?;
    if !result.status.success() {
        return None;
    }
    Some(PathBuf::from(String::from_utf8_lossy(&result.stdout).trim()))
}

pub async fn detach(path: &str) {
    let _ = tokio::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["config", "--local", "--unset-all", "guaca.githubConnection"])
        .output()
        .await;
}

/// Each gh invocation obtains a fresh-enough token. No 45-minute job is tied
/// to a token captured at process startup. Other repositories retain normal gh.
pub async fn environment(path: &str, command: &mut tokio::process::Command) {
    let Some(file) = attached(path).await else {
        return;
    };
    let Some(parent) = file.parent() else {
        return;
    };
    let wrapper = parent.join("github-bin");
    let current = std::env::var_os("PATH").unwrap_or_default();
    let paths: Vec<_> = std::env::split_paths(&current).filter(|p| p != &wrapper).collect();
    if let Some(real) = paths.iter().map(|p| p.join("gh")).find(|p| p.is_file()) {
        command.env("GUACA_GH_BINARY", real);
    }
    if let Ok(path) = std::env::join_paths(std::iter::once(wrapper).chain(paths)) {
        command.env("PATH", path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn app_access_has_one_exact_github_repository() {
        assert_eq!(repository("https://github.com/Owner/Repo.git").unwrap(), "owner/repo");
        for remote in [
            "https://github.com.evil.test/o/r",
            "https://github.com/o/r/extra",
            "https://github.com/o/r?token=secret",
            "http://github.com/o/r",
            "https://github.com/o/%72",
            "git@github.com:o/r",
        ] {
            assert!(repository(remote).is_err(), "{remote}");
        }
    }
}
