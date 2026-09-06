//! Saved Git credentials, addressed by opaque IDs rather than client-chosen paths.
//! Only descriptions leave this module. Reuse makes a new repository-scoped
//! entry so rotating or removing one repository's access cannot change another.
//! The current adapter is Git's private plaintext store, not an encrypted vault.

use std::{path::Path, time::SystemTime};

use super::{auth, RepoError};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub id: String,
    pub remote: String,
    pub username: String,
}

// Deliberately neither Debug nor Serialize: errors and IPC carry Saved only.
struct Credential {
    remote: reqwest::Url,
    username: String,
    token: String,
}

fn unavailable() -> RepoError {
    RepoError::Connection(
        "Saved Git credential is unavailable. Choose another credential or enter a new token"
            .into(),
    )
}

async fn read(directory: &Path, id: &str) -> Result<Credential, RepoError> {
    let id = uuid::Uuid::parse_str(id).map_err(|_| unavailable())?.to_string();
    let file = directory.join(id);
    let metadata = tokio::fs::symlink_metadata(&file).await.map_err(|_| unavailable())?;
    if !metadata.is_file() || metadata.len() > 65536 {
        return Err(unavailable());
    }
    let raw = tokio::fs::read_to_string(file).await.map_err(|_| unavailable())?;
    let decode = |value: &str| {
        percent_encoding::percent_decode_str(value)
            .decode_utf8()
            .map(|value| value.into_owned())
            .map_err(|_| unavailable())
    };
    // Git rewrites successful credentials and percent-encodes the authority,
    // including a port's colon and IPv6 brackets. Decode that component before
    // asking a URL parser to interpret the remote; never decode the whole URL.
    let (scheme, rest) = raw.trim().split_once("://").ok_or_else(unavailable)?;
    let (authority, path) = rest.split_once('/').ok_or_else(unavailable)?;
    let (userinfo, host) = authority.rsplit_once('@').ok_or_else(unavailable)?;
    let host = decode(host)?;
    if host.chars().any(|c| c.is_control() || matches!(c, '/' | '?' | '#' | '@' | '\\')) {
        return Err(unavailable());
    }
    let mut remote = reqwest::Url::parse(&format!("{scheme}://{userinfo}@{host}/{path}"))
        .map_err(|_| unavailable())?;
    let username = decode(remote.username())?;
    let token = decode(remote.password().ok_or_else(unavailable)?)?;
    if username.is_empty() || token.is_empty() || raw.trim().contains(['\r', '\n']) {
        return Err(unavailable());
    }
    remote.set_username("").map_err(|_| unavailable())?;
    remote.set_password(None).map_err(|_| unavailable())?;
    auth::https_remote(remote.as_str())?;
    Ok(Credential { remote, username, token })
}

/// Invalid and SSH URLs have no token choices. Actual reuse validates again,
/// because a client can change its URL after loading this list.
pub async fn list(directory: &Path, remote: &str) -> Result<Vec<Saved>, RepoError> {
    let Ok(remote) = auth::https_remote(remote.trim()) else { return Ok(vec![]) };
    let mut entries = match tokio::fs::read_dir(directory).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(_) => {
            return Err(RepoError::Connection(
                "Could not list saved Git credentials. Check backend filesystem permissions".into(),
            ))
        }
    };
    let mut saved = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(|_| unavailable())? {
        let id = entry.file_name().to_string_lossy().into_owned();
        // Ignore pending writes, GitHub App metadata, removed or malformed entries.
        let Ok(credential) = read(directory, &id).await else { continue };
        if credential.remote.origin() == remote.origin() {
            let modified = entry
                .metadata()
                .await
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            saved.push((
                modified,
                Saved { id, remote: credential.remote.to_string(), username: credential.username },
            ));
        }
    }
    saved.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
    Ok(saved.into_iter().map(|(_, saved)| saved).collect())
}

/// Check the complete origin, including scheme and port, before writing or
/// contacting Git. A saved credential never follows the client to a new host.
async fn for_remote(directory: &Path, id: &str, remote: &str) -> Result<Credential, RepoError> {
    let destination = auth::https_remote(remote)?;
    let credential = read(directory, id).await?;
    if destination.origin() != credential.remote.origin() {
        return Err(RepoError::Connection("This saved credential belongs to a different Git server. Choose a credential for this server or enter a new token".into()));
    }
    Ok(credential)
}

pub async fn keep(directory: &Path, id: &str, file: &Path, remote: &str) -> Result<(), RepoError> {
    let credential = for_remote(directory, id, remote).await?;
    auth::keep(file, remote, &credential.username, &credential.token).await
}

pub async fn set(
    directory: &Path,
    id: &str,
    file: &Path,
    path: &str,
) -> Result<auth::Connection, RepoError> {
    let remote = auth::origin(path, false).await?.ok_or_else(unavailable)?;
    let credential = for_remote(directory, id, &remote).await?;
    auth::set_for_remote(path, file, &remote, &credential.username, &credential.token).await
}
