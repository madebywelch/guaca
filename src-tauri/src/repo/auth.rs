//! Repository-scoped Git credentials. The CLI and its linked worktrees read
//! the same helper; no token is returned over IPC or placed in a remote URL.

use std::{path::Path, process::Stdio, time::Duration};

use super::RepoError;
use crate::domain::repository::GitIdentity;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub remote: Option<String>,
    pub push_remote: Option<String>,
    pub managed_credential: bool,
    pub accepts_token: bool,
    pub github_app: bool,
    pub github_available: bool,
    pub author: GitIdentity,
}

fn error(message: &str) -> RepoError {
    RepoError::Connection(message.into())
}

/// Do not reflect userinfo, query strings, or malformed addresses in errors.
pub fn https_remote(remote: &str) -> Result<reqwest::Url, RepoError> {
    let url = reqwest::Url::parse(remote).map_err(|_| {
        error("Git tokens need an https:// origin; configure a backend SSH key for SSH remotes")
    })?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if (url.scheme() != "https" && !(url.scheme() == "http" && loopback))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() == "/"
    {
        return Err(error("Use an HTTPS repository URL without embedded credentials, query or fragment (HTTP is allowed only on loopback)"));
    }
    Ok(url)
}

pub async fn keep(file: &Path, remote: &str, username: &str, token: &str) -> Result<(), RepoError> {
    let url = https_remote(remote)?;
    let username = if username.trim().is_empty() { "git" } else { username.trim() };
    let token = token.trim();
    if token.is_empty() {
        return Err(error("Enter a repository access token"));
    }
    let parent = file.parent().ok_or_else(|| error("No credential directory"))?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|_| error("Could not create the credential directory"))?;
    let authority = url.as_str().split_once("://").unwrap().1.split('/').next().unwrap();
    let line = format!(
        "{}://{}:{}@{}{}\n",
        url.scheme(),
        super::urlencode(username),
        super::urlencode(token),
        authority,
        url.path()
    );
    // Atomic replacement also fixes the permissions of an existing file.
    // A unique sibling prevents concurrent writes from sharing a partial file.
    let pending = parent.join(format!(".{}", uuid::Uuid::new_v4()));
    let write = async {
        use tokio::io::AsyncWriteExt;
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut out = options.open(&pending).await?;
        out.write_all(line.as_bytes()).await?;
        out.sync_all().await?;
        drop(out);
        tokio::fs::rename(&pending, file).await
    }
    .await;
    if write.is_err() {
        let _ = tokio::fs::remove_file(&pending).await;
        return Err(error("Could not save the repository credential"));
    }
    Ok(())
}

pub fn helper(file: &Path) -> String {
    // Git runs credential helpers through a shell, including on paths with
    // spaces or apostrophes. Shell-quote only the path, never the token.
    format!("store --file='{}'", file.to_string_lossy().replace('\'', "'\\''"))
}

async fn git(path: &str, args: &[&str]) -> Result<std::process::Output, RepoError> {
    let mut command = tokio::process::Command::new("git");
    command
        .arg("-C")
        .arg(path)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("SSH_ASKPASS", "")
        .env("GIT_SSH_COMMAND", "ssh -oBatchMode=yes -oConnectTimeout=10")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    tokio::time::timeout(Duration::from_secs(30), command.output())
        .await
        .map_err(|_| error("Git connection check timed out after 30 seconds"))?
        .map_err(|_| error("Could not run git on the backend"))
}

async fn setting(path: &str, args: &[&str]) -> Result<(), RepoError> {
    if git(path, args).await?.status.success() {
        Ok(())
    } else {
        Err(error("Could not update this repository's Git credential helper"))
    }
}

async fn origin(path: &str, push: bool) -> Result<Option<String>, RepoError> {
    let args = if push {
        vec!["remote", "get-url", "--push", "origin"]
    } else {
        vec!["remote", "get-url", "origin"]
    };
    let result = git(path, &args).await?;
    Ok(result.status.success().then(|| String::from_utf8_lossy(&result.stdout).trim().into()))
}

fn shown(remote: Option<String>) -> Option<String> {
    remote.map(|raw| {
        if let Ok(mut url) = reqwest::Url::parse(&raw) {
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.set_query(None);
            url.set_fragment(None);
            url.to_string()
        } else {
            raw
        }
    })
}

/// Validate both fields before touching Git config. Identity is public commit
/// metadata, not a credential, and is never inferred from the installation owner.
pub fn validate_identity(author: &GitIdentity) -> Result<(), RepoError> {
    let name = author.name.trim();
    let email = author.email.trim();
    if name.is_empty()
        || name.len() > 256
        || name.chars().any(|c| c.is_control() || matches!(c, '<' | '>'))
        || email.len() > 254
        || email.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>'))
        || !email.split_once('@').is_some_and(|(local, host)| {
            !local.is_empty() && !host.is_empty() && !host.contains('@')
        })
    {
        return Err(error("Enter your commit name and email address. Use an email linked to your Git account or its noreply address"));
    }
    Ok(())
}

pub async fn identity(path: &str) -> Result<GitIdentity, RepoError> {
    async fn value(path: &str, key: &str) -> Result<String, RepoError> {
        let result = git(path, &["config", "--get", key]).await?;
        match result.status.code() {
            Some(0) => Ok(String::from_utf8_lossy(&result.stdout).trim().into()),
            Some(1) => Ok(String::new()),
            _ => Err(error("Could not read this repository's commit identity")),
        }
    }
    Ok(GitIdentity {
        name: value(path, "user.name").await?,
        email: value(path, "user.email").await?,
    })
}

pub async fn set_identity(path: &str, author: &GitIdentity) -> Result<(), RepoError> {
    validate_identity(author)?;
    for (key, value) in [
        ("user.name", author.name.trim()),
        ("user.email", author.email.trim()),
        ("user.useConfigOnly", "true"),
    ] {
        if !git(path, &["config", "--local", "--replace-all", key, value]).await?.status.success() {
            return Err(error("Could not save this repository's commit identity. Check backend filesystem permissions"));
        }
    }
    Ok(())
}

pub async fn connection(path: &str, file: &Path) -> Result<Connection, RepoError> {
    let remote = origin(path, false).await?;
    let push_remote = origin(path, true).await?;
    let accepts_token = remote.as_deref().is_some_and(|r| https_remote(r).is_ok());
    Ok(Connection {
        author: identity(path).await?,
        remote: shown(remote.clone()),
        push_remote: shown(push_remote),
        github_app: match super::github::attached(path).await {
            Some(file) => file.is_file(),
            None => false,
        },
        github_available: super::github::configured()
            && remote.as_deref().is_some_and(|r| super::github::repository(r).is_ok()),
        accepts_token,
        managed_credential: tokio::fs::metadata(file)
            .await
            .is_ok_and(|metadata| metadata.len() > 0),
    })
}

pub async fn set(
    path: &str,
    file: &Path,
    username: &str,
    token: &str,
) -> Result<Connection, RepoError> {
    let remote =
        origin(path, false).await?.ok_or_else(|| error("This repository has no origin remote"))?;
    keep(file, &remote, username, token).await?;
    // Reset inherited helpers, which could otherwise return the wrong account
    // before this repository's helper is consulted. No global config changes.
    setting(path, &["config", "--local", "--replace-all", "credential.helper", ""]).await?;
    setting(path, &["config", "--local", "--add", "credential.helper", &helper(file)]).await?;
    setting(path, &["config", "--local", "credential.useHttpPath", "true"]).await?;
    super::github::detach(path).await;
    let _ = tokio::fs::remove_file(super::github::file(file)).await;
    connection(path, file).await
}

pub async fn clear(path: &str, file: &Path) -> Result<Connection, RepoError> {
    if super::github::attached(path).await.is_some() {
        match tokio::fs::remove_file(super::github::file(file)).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(error("Could not remove GitHub App connection")),
        }
    }
    match tokio::fs::remove_file(file).await {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(error("Could not remove the saved Git credential")),
    }
    // Keep the empty helper configured: removing it could silently fall back
    // to another account inherited from the backend's global configuration.
    connection(path, file).await
}

/// Read access and a receive-pack handshake without changing remote refs.
/// A dry run cannot prove branch protection or server-side hooks will accept
/// an actual update, so the returned sentence states that limit explicitly.
pub async fn check(path: &str) -> Result<String, RepoError> {
    let read = git(path, &["ls-remote", "--", "origin"]).await?;
    if !read.status.success() {
        return Err(error("Git read check failed. Check origin, backend network access and the repository credential"));
    }
    let head = git(path, &["rev-parse", "--verify", "HEAD"]).await?;
    if !head.status.success() {
        return Ok("Read access works. Commit a file before checking push access.".into());
    }
    let reference = format!("HEAD:refs/heads/guaca-access-check-{}", uuid::Uuid::new_v4());
    let push = git(
        path,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "push",
            "--dry-run",
            "--no-verify",
            "--",
            "origin",
            &reference,
        ],
    )
    .await?;
    if !push.status.success() {
        return Err(error("Git read access works, but the push dry run failed. Check write permissions and the push remote"));
    }
    Ok("Read access and push dry run succeeded. No remote refs changed; branch protection and server hooks are checked only on an actual push.".into())
}
