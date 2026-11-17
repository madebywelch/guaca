use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use sha2::{Digest, Sha256};

use crate::domain::ids::AgentId;

static MEMORY_LOCK: Mutex<()> = Mutex::new(());

pub fn version(content: &str) -> String {
    format!("{:x}", Sha256::digest(content.as_bytes()))
}

pub const MAX_MEMORY: usize = 16_000;

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("could not access the workspace at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut last_dash = true;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
        if out.chars().count() >= 32 {
            break;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "agent".to_string()
    } else {
        trimmed
    }
}

#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn preferred_path(&self, id: AgentId, name: &str) -> PathBuf {
        self.root.join(format!("{}-{}.md", slug(name), id.short()))
    }

    fn existing_path(&self, id: AgentId) -> io::Result<Option<PathBuf>> {
        let suffix = format!("-{}.md", id.short());
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err),
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(&suffix)) {
                return Ok(Some(path));
            }
        }
        Ok(None)
    }

    pub fn read(&self, id: AgentId) -> String {
        let _guard = MEMORY_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        self.read_unlocked(id)
    }

    pub fn read_versioned(&self, id: AgentId) -> MemorySnapshot {
        let _guard = MEMORY_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let content = self.read_unlocked(id);
        MemorySnapshot { version: version(&content), content }
    }

    fn read_unlocked(&self, id: AgentId) -> String {
        self.existing_path(id)
            .ok()
            .flatten()
            .and_then(|path| fs::read_to_string(path).ok())
            .unwrap_or_default()
    }

    pub fn write(&self, id: AgentId, name: &str, content: &str) -> Result<Stored, WorkspaceError> {
        match self.write_versioned(id, name, content, None)? {
            WriteResult::Stored(stored) => Ok(stored),
            WriteResult::Conflict { .. } => unreachable!(),
        }
    }

    pub fn write_versioned(
        &self,
        id: AgentId,
        name: &str,
        content: &str,
        expected: Option<&str>,
    ) -> Result<WriteResult, WorkspaceError> {
        self.write_through(id, name, content, expected, |from, to| fs::rename(from, to))
    }

    fn write_through(
        &self,
        id: AgentId,
        name: &str,
        content: &str,
        expected: Option<&str>,
        rename: fn(&Path, &Path) -> io::Result<()>,
    ) -> Result<WriteResult, WorkspaceError> {
        let _guard = MEMORY_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let before = self.read_unlocked(id);
        if expected.is_some_and(|expected| expected != version(&before)) {
            return Ok(WriteResult::Conflict { current: before, draft: content.to_string() });
        }
        fs::create_dir_all(&self.root)
            .map_err(|source| WorkspaceError::Io { path: self.root.clone(), source })?;

        let trimmed = content.trim();
        let (body, truncated) = if trimmed.chars().count() <= MAX_MEMORY {
            (trimmed.to_string(), false)
        } else {
            let mut kept: String = trimmed.chars().take(MAX_MEMORY).collect();
            if let Some(last_break) = kept.rfind('\n') {
                if last_break > MAX_MEMORY / 2 {
                    kept.truncate(last_break);
                }
            }
            (kept, true)
        };

        let target = self.preferred_path(id, name);
        let current = self.existing_path(id).ok().flatten();
        if let Some(current) = current {
            if current != target {
                let _ = fs::rename(&current, &target);
            }
        }

        replace(&target, &body, rename)
            .map_err(|source| WorkspaceError::Io { path: target.clone(), source })?;

        Ok(WriteResult::Stored(Stored {
            before,
            characters: body.chars().count(),
            content: body,
            truncated,
            path: target,
        }))
    }

    pub fn remove(&self, id: AgentId) -> Result<(), WorkspaceError> {
        let _guard = MEMORY_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let path = self
            .existing_path(id)
            .map_err(|source| WorkspaceError::Io { path: self.root.clone(), source })?;
        let Some(path) = path else {
            return Ok(());
        };
        match fs::remove_file(&path) {
            Err(source) if source.kind() != io::ErrorKind::NotFound => {
                Err(WorkspaceError::Io { path, source })
            }
            _ => Ok(()),
        }
    }
}

fn replace(
    target: &Path,
    body: &str,
    rename: fn(&Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    let temp = target.with_extension("md.tmp");
    let written = fs::File::create(&temp)
        .and_then(|mut file| file.write_all(body.as_bytes()).and_then(|()| file.sync_all()));
    let placed = written.and_then(|()| rename(&temp, target));
    if placed.is_err() {
        let _ = fs::remove_file(&temp);
    }
    placed
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stored {
    pub before: String,
    pub content: String,
    pub characters: usize,
    pub truncated: bool,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WriteResult {
    Stored(Stored),
    Conflict { current: String, draft: String },
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MemorySnapshot {
    pub content: String,
    pub version: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> (Workspace, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        (Workspace::new(dir.path().join("workspace")), dir)
    }

    #[test]
    fn memory_round_trips() {
        let (ws, _dir) = workspace();
        let id = AgentId::new();
        assert_eq!(ws.read(id), "", "an agent starts with nothing");

        ws.write(id, "Manager", "# Style\nTerse. No preamble.").unwrap();
        assert_eq!(ws.read(id), "# Style\nTerse. No preamble.");
    }

    #[test]
    fn a_rewrite_replaces_rather_than_appends() {
        let (ws, _dir) = workspace();
        let id = AgentId::new();
        ws.write(id, "Manager", "first belief").unwrap();
        ws.write(id, "Manager", "corrected belief").unwrap();
        assert_eq!(ws.read(id), "corrected belief");
    }

    #[test]
    fn a_write_hands_back_the_version_it_replaced() {
        let (ws, _dir) = workspace();
        let id = AgentId::new();

        let first = ws.write(id, "Manager", "first belief").unwrap();
        assert_eq!(first.before, "", "an agent's first memory replaced nothing");

        let second = ws.write(id, "Manager", "corrected belief").unwrap();
        assert_eq!(second.before, "first belief");
    }

    #[test]
    fn the_replaced_version_survives_a_rename() {
        let (ws, _dir) = workspace();
        let id = AgentId::new();
        ws.write(id, "Manager", "kept").unwrap();

        let stored = ws.write(id, "Coordinator", "revised").unwrap();
        assert_eq!(stored.before, "kept");
    }

    #[test]
    fn the_file_is_named_after_the_agent() {
        let (ws, _dir) = workspace();
        let id = AgentId::new();
        let stored = ws.write(id, "Head Chef!", "x").unwrap();
        let name = stored.path.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("head-chef-"), "expected a readable name, got {name}");
        assert!(name.ends_with(".md"));
    }

    #[test]
    fn renaming_an_agent_moves_its_memory_without_losing_it() {
        let (ws, _dir) = workspace();
        let id = AgentId::new();
        ws.write(id, "Manager", "remembered").unwrap();
        let stored = ws.write(id, "Coordinator", "remembered").unwrap();

        assert!(stored.path.file_name().unwrap().to_str().unwrap().starts_with("coordinator-"));
        assert_eq!(ws.read(id), "remembered");
        assert_eq!(fs::read_dir(ws.root()).unwrap().count(), 1);
    }

    #[test]
    fn memory_is_found_by_id_even_if_the_filename_is_stale() {
        let (ws, _dir) = workspace();
        let id = AgentId::new();
        ws.write(id, "Manager", "kept").unwrap();

        let stale = ws.root().join(format!("something-else-{}.md", id.short()));
        fs::rename(ws.preferred_path(id, "Manager"), &stale).unwrap();
        assert_eq!(ws.read(id), "kept");
    }

    #[test]
    fn two_agents_never_share_a_file() {
        let (ws, _dir) = workspace();
        let (a, b) = (AgentId::new(), AgentId::new());
        ws.write(a, "Manager", "mine").unwrap();
        ws.write(b, "Manager", "also mine").unwrap();

        assert_eq!(ws.read(a), "mine");
        assert_eq!(ws.read(b), "also mine");
    }

    #[test]
    fn oversized_memory_is_cut_and_reported() {
        let (ws, _dir) = workspace();
        let id = AgentId::new();
        let huge = "a line of notes\n".repeat(MAX_MEMORY / 8);

        let stored = ws.write(id, "Manager", &huge).unwrap();
        assert!(stored.truncated);
        assert!(stored.characters <= MAX_MEMORY);
        assert!(ws.read(id).chars().count() <= MAX_MEMORY);
    }

    #[test]
    fn truncation_lands_on_a_line_boundary() {
        let (ws, _dir) = workspace();
        let id = AgentId::new();
        ws.write(id, "Manager", &"a line of notes\n".repeat(MAX_MEMORY / 8)).unwrap();
        assert!(ws.read(id).ends_with("a line of notes"), "cut mid-sentence");
    }

    #[test]
    fn memory_within_the_cap_is_stored_whole() {
        let (ws, _dir) = workspace();
        let id = AgentId::new();
        let body = "x".repeat(MAX_MEMORY);
        let stored = ws.write(id, "Manager", &body).unwrap();
        assert!(!stored.truncated);
        assert_eq!(ws.read(id).chars().count(), MAX_MEMORY);
    }

    #[test]
    fn a_write_that_fails_before_its_rename_leaves_the_old_page() {
        let (ws, _dir) = workspace();
        let id = AgentId::new();
        ws.write(id, "Manager", "what it knew").unwrap();

        let failed = ws.write_through(id, "Manager", "what it meant to write", None, |_, _| {
            Err(io::Error::other("stopped before the rename"))
        });
        assert!(failed.is_err(), "the write did not fail, so this proves nothing");
        assert_eq!(ws.read(id), "what it knew");
        assert_eq!(
            fs::read_dir(ws.root()).unwrap().count(),
            1,
            "the half-made page was left lying beside the real one"
        );

        ws.write(id, "Manager", "what it meant to write").unwrap();
        assert_eq!(ws.read(id), "what it meant to write", "and the next write goes through");
    }

    #[test]
    fn a_page_left_half_written_by_a_crash_is_never_read_as_memory() {
        let (ws, _dir) = workspace();
        let id = AgentId::new();
        let stored = ws.write(id, "Manager", "what it knew").unwrap();
        fs::write(stored.path.with_extension("md.tmp"), "what it me").unwrap();

        assert_eq!(ws.read(id), "what it knew");
        ws.write(id, "Manager", "what it meant to write").unwrap();
        assert_eq!(ws.read(id), "what it meant to write");
    }

    #[test]
    fn deleting_an_agent_removes_its_memory() {
        let (ws, _dir) = workspace();
        let id = AgentId::new();
        ws.write(id, "Manager", "gone soon").unwrap();
        ws.remove(id).unwrap();
        assert_eq!(ws.read(id), "");
    }

    #[test]
    fn memory_that_cannot_be_removed_is_an_error_rather_than_a_success() {
        let (ws, _dir) = workspace();
        let id = AgentId::new();
        ws.write(id, "Manager", "kept").unwrap();
        let path = ws.preferred_path(id, "Manager");
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        fs::write(path.join("in-the-way"), "a folder where the memory was").unwrap();

        assert!(ws.remove(id).is_err());
        assert!(path.exists());
    }

    #[test]
    fn removing_memory_that_is_already_gone_succeeds_so_a_retry_converges() {
        let (ws, _dir) = workspace();
        let id = AgentId::new();
        ws.remove(id).unwrap();
        ws.write(id, "Manager", "gone soon").unwrap();
        ws.remove(id).unwrap();
        ws.remove(id).unwrap();
    }

    #[test]
    fn reading_a_missing_workspace_is_empty_rather_than_an_error() {
        let (ws, _dir) = workspace();
        assert_eq!(ws.read(AgentId::new()), "", "nothing has been written yet");
    }

    #[test]
    fn slugs_are_safe_and_recognizable() {
        assert_eq!(slug("Manager"), "manager");
        assert_eq!(slug("Head Chef"), "head-chef");
        assert_eq!(slug("  ../../etc/passwd  "), "etc-passwd");
        assert_eq!(slug("!!!"), "agent");
        assert_eq!(slug(""), "agent");
        assert!(slug(&"x".repeat(200)).chars().count() <= 32);
    }

    #[test]
    fn a_hostile_name_cannot_escape_the_workspace() {
        let (ws, _dir) = workspace();
        let id = AgentId::new();
        let stored = ws.write(id, "../../../../tmp/pwned", "x").unwrap();
        assert!(
            stored.path.starts_with(ws.root()),
            "memory escaped the workspace: {}",
            stored.path.display()
        );
    }
}
