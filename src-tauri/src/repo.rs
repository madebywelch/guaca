//! Whether a directory is one an agent may be given.
//!
//! One question, asked by running git rather than by looking for a `.git`
//! entry. A work tree that is a linked worktree, a submodule, or a checkout
//! whose `.git` is a file rather than a directory all answer correctly to git
//! and all fool a directory check, and each of those is somebody's ordinary
//! setup rather than an exotic case. Asking git also answers a second question
//! for free: whether git is installed at all, which whatever works in the
//! directory later is going to need.
//!
//! This is where a repository stops being a string. Everything above it in
//! [`crate::domain::repository`] is the shape and the rules that need no disk;
//! this is the disk.
//!
//! ## The directory is on this machine
//!
//! Not on an agent's sandbox. The operator picks a directory they already have,
//! with work already in it, and that is the whole point of the feature: an
//! agent that can only see a fresh clone cannot see the branch you are on, the
//! change you have not committed, or the toolchain your repository actually
//! builds with.
//!
//! What makes that defensible is not this check. It is the three things around
//! it: the operator picks the path and no model ever supplies one, git is the
//! undo, and the process that works in there is confined to it. This function
//! is the first of the three and the cheapest, and it must never be described
//! as the boundary.

use std::path::Path;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum RepoError {
    #[error("`{0}` is not a directory on this machine; pick one that exists")]
    NotADirectory(String),
    #[error(
        "git is not installed, or is not on this app's PATH. Guaca needs it to work in a \
         repository at all: install the Xcode command line tools with `xcode-select --install`, \
         or install git, and link the directory again"
    )]
    GitMissing,
    #[error(
        "`{0}` is not a git repository. An agent works there only because git can undo it, so \
         run `git init` in it first, or pick a directory that already has a repository"
    )]
    NotARepository(String),
    #[error(
        "`{linked}` is inside the repository at `{root}` rather than being its root. Link \
         `{root}` instead: what an agent may write to and what git can undo have to be the same \
         directory. To keep an agent to part of it, say so in the repository's note"
    )]
    NotTheRoot { linked: String, root: String },
    #[error("`{path}` could not be read ({reason})")]
    Unreadable { path: String, reason: String },
}

/// The canonical path of the work tree this directory is the root of.
///
/// Canonical because the answer is stored and compared: a path reached through
/// a symlink and the same path reached directly are one directory, and two rows
/// for it would be two reaches over one tree with nothing saying which applied.
pub async fn verify(path: &str) -> Result<String, RepoError> {
    let given = Path::new(path);

    let canonical = tokio::fs::canonicalize(given).await.map_err(|err| match err.kind() {
        std::io::ErrorKind::NotFound => RepoError::NotADirectory(path.to_string()),
        _ => RepoError::Unreadable { path: path.to_string(), reason: err.to_string() },
    })?;
    if !tokio::fs::metadata(&canonical).await.map(|meta| meta.is_dir()).unwrap_or(false) {
        return Err(RepoError::NotADirectory(path.to_string()));
    }

    let output = tokio::process::Command::new("git")
        .arg("-C")
        .arg(&canonical)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .await
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => RepoError::GitMissing,
            _ => RepoError::Unreadable { path: path.to_string(), reason: err.to_string() },
        })?;

    let shown = String::from_utf8_lossy(&output.stdout).trim().to_string();
    // A bare repository exits zero and prints nothing: there is no work tree,
    // so there is nothing to open and nothing to edit. Reported as not a
    // repository rather than as an empty answer, because that is what it is
    // from where the operator is standing.
    if !output.status.success() || shown.is_empty() {
        return Err(RepoError::NotARepository(display(&canonical)));
    }

    // Canonicalized on both sides before they are compared. On macOS a
    // directory under `/tmp` or `/var` is reached through a symlink, and git
    // resolves it while the operator's own path does not: comparing the two as
    // typed reports the root of a repository as being inside itself.
    let root = tokio::fs::canonicalize(&shown)
        .await
        .map_err(|err| RepoError::Unreadable { path: shown.clone(), reason: err.to_string() })?;

    if root != canonical {
        return Err(RepoError::NotTheRoot { linked: display(&canonical), root: display(&root) });
    }

    Ok(display(&canonical))
}

/// A path as the operator would read it back.
///
/// Lossy on purpose: a path this app cannot spell is one the operator cannot
/// act on either, and refusing to draw it would leave them with an error naming
/// nothing.
fn display(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// A real repository, made here rather than mocked. The whole value of this
    /// function is that it agrees with git, and a stub agrees with itself.
    async fn a_repository(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("guac-repo-{name}-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&root).await;
        tokio::fs::create_dir_all(&root).await.unwrap();
        let done = tokio::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .arg("init")
            .output()
            .await
            .expect("git has to be installed to run this suite");
        assert!(done.status.success(), "git init failed: {done:?}");
        tokio::fs::canonicalize(&root).await.unwrap()
    }

    #[tokio::test]
    async fn a_work_tree_root_is_accepted_and_comes_back_canonical() {
        let root = a_repository("root").await;
        let verified = verify(root.to_str().unwrap()).await.unwrap();
        assert_eq!(verified, root.to_string_lossy());
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn a_subdirectory_is_refused_and_names_the_root() {
        let root = a_repository("sub").await;
        let inner = root.join("packages").join("api");
        tokio::fs::create_dir_all(&inner).await.unwrap();

        let err = verify(inner.to_str().unwrap()).await.unwrap_err();
        match &err {
            RepoError::NotTheRoot { root: named, .. } => {
                assert_eq!(named, &root.to_string_lossy().to_string())
            }
            other => panic!("expected the root to be named, got {other:?}"),
        }
        // The refusal is the fix: an operator reading it knows which directory
        // to link without going and looking.
        assert!(err.to_string().contains(&root.to_string_lossy().to_string()));
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn a_directory_with_no_repository_is_refused_with_the_command_that_fixes_it() {
        let plain = std::env::temp_dir().join(format!("guac-plain-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&plain).await;
        tokio::fs::create_dir_all(&plain).await.unwrap();

        let err = verify(plain.to_str().unwrap()).await.unwrap_err();
        assert!(matches!(err, RepoError::NotARepository(_)), "got {err:?}");
        assert!(err.to_string().contains("git init"), "the refusal has to say what to do");
        let _ = tokio::fs::remove_dir_all(&plain).await;
    }

    #[tokio::test]
    async fn a_path_that_is_not_there_is_refused_before_git_is_asked() {
        let err = verify("/no/such/directory/anywhere").await.unwrap_err();
        assert!(matches!(err, RepoError::NotADirectory(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn a_file_is_not_a_directory() {
        let file = std::env::temp_dir().join(format!("guac-file-{}", std::process::id()));
        tokio::fs::write(&file, b"not a directory").await.unwrap();
        let err = verify(file.to_str().unwrap()).await.unwrap_err();
        assert!(matches!(err, RepoError::NotADirectory(_)), "got {err:?}");
        let _ = tokio::fs::remove_file(&file).await;
    }
}
