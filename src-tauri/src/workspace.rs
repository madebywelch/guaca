//! Per-agent memory on disk.
//!
//! Each agent owns one small markdown file that it can rewrite, and that is
//! shown to it at the start of every turn. This is the smallest thing that
//! deserves the name memory, and the shape is deliberate.
//!
//! It is called memory everywhere a person or a model reads about it, and
//! `notes` everywhere the code names it: the tool is `update_notes`, the
//! commands are `agent_notes` and `set_agent_notes`, and the files are these.
//! Renaming the internals would rewrite the IPC contract to no effect, so
//! instead both words reach the same file at the model boundary, and the
//! operator's word is the one an agent is told.
//!
//! The 2026 survey of agent memory (arXiv 2603.07670) frames memory as a
//! write-manage-read loop and names the engineering realities that decide
//! whether it works: write-path filtering, contradiction handling, continual
//! consolidation, and learned forgetting. Three design choices here follow
//! directly from that:
//!
//! - **Always resident, never retrieved.** The file is small enough to sit in
//!   the prompt, so there is no retrieval step to get wrong and no relevance
//!   model to tune. Retrieval machinery earns its place at a scale this does
//!   not reach.
//! - **Replace, never append.** The only write is a full rewrite, which forces
//!   the agent to reconcile what it already believed against what it just
//!   learned. Append-only turns into the transcript it was supposed to
//!   summarize.
//! - **A hard cap.** Forgetting has to be someone's decision. A ceiling makes
//!   the agent choose what survives rather than letting the file grow until it
//!   crowds out the conversation.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::domain::ids::AgentId;

/// Notes longer than this are refused. Roughly a page: enough for a persona,
/// standing preferences, and a handful of durable facts.
pub const MAX_NOTES: usize = 4_000;

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("could not access the workspace at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Turns an agent name into something safe and recognizable in a file listing.
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

    /// Where an agent's notes should live, given its current name.
    pub fn preferred_path(&self, id: AgentId, name: &str) -> PathBuf {
        self.root.join(format!("{}-{}.md", slug(name), id.short()))
    }

    /// Finds an agent's file wherever it currently is.
    ///
    /// Located by the id suffix rather than by name, so renaming an agent can
    /// never orphan its notes even if the file move fails.
    fn existing_path(&self, id: AgentId) -> Option<PathBuf> {
        let suffix = format!("-{}.md", id.short());
        let entries = fs::read_dir(&self.root).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(&suffix)) {
                return Some(path);
            }
        }
        None
    }

    /// An agent's notes, or an empty string if it has never written any.
    pub fn read(&self, id: AgentId) -> String {
        self.existing_path(id).and_then(|path| fs::read_to_string(path).ok()).unwrap_or_default()
    }

    /// Replaces an agent's notes.
    ///
    /// Returns what was actually stored, which is the input trimmed and, if it
    /// ran over the cap, cut at a line boundary with a marker. Silently storing
    /// a truncated file would let an agent believe it had recorded something it
    /// had not.
    ///
    /// It also returns what was there first. Replacing is the only write this
    /// interface has, so the version it replaced exists nowhere else a moment
    /// later, and this is the only place it can be read without a race: the
    /// same memory is written from every thread an agent holds and edited by
    /// the operator by hand, so anything that reads the file afterward is
    /// reading somebody else's write.
    pub fn write(&self, id: AgentId, name: &str, content: &str) -> Result<Stored, WorkspaceError> {
        fs::create_dir_all(&self.root)
            .map_err(|source| WorkspaceError::Io { path: self.root.clone(), source })?;

        let trimmed = content.trim();
        let (body, truncated) = if trimmed.chars().count() <= MAX_NOTES {
            (trimmed.to_string(), false)
        } else {
            let mut kept: String = trimmed.chars().take(MAX_NOTES).collect();
            // Cut at a line boundary so the file never ends mid-sentence.
            if let Some(last_break) = kept.rfind('\n') {
                if last_break > MAX_NOTES / 2 {
                    kept.truncate(last_break);
                }
            }
            (kept, true)
        };

        let target = self.preferred_path(id, name);
        let current = self.existing_path(id);
        let before =
            current.as_ref().and_then(|path| fs::read_to_string(path).ok()).unwrap_or_default();
        // Renaming is cosmetic: lookup is by id, so a failure here is harmless.
        if let Some(current) = current {
            if current != target {
                let _ = fs::rename(&current, &target);
            }
        }

        fs::write(&target, &body)
            .map_err(|source| WorkspaceError::Io { path: target.clone(), source })?;

        Ok(Stored { before, characters: body.chars().count(), truncated, path: target })
    }

    /// Deletes an agent's notes. Called when the agent is deleted.
    pub fn remove(&self, id: AgentId) {
        if let Some(path) = self.existing_path(id) {
            let _ = fs::remove_file(path);
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stored {
    /// The version this write replaced, empty where there was none.
    pub before: String,
    pub characters: usize,
    pub truncated: bool,
    pub path: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> (Workspace, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        (Workspace::new(dir.path().join("workspace")), dir)
    }

    #[test]
    fn notes_round_trip() {
        let (ws, _dir) = workspace();
        let id = AgentId::new();
        assert_eq!(ws.read(id), "", "an agent starts with nothing");

        ws.write(id, "Manager", "# Style\nTerse. No preamble.").unwrap();
        assert_eq!(ws.read(id), "# Style\nTerse. No preamble.");
    }

    #[test]
    fn a_rewrite_replaces_rather_than_appends() {
        // The whole interface: replacing forces the agent to reconcile instead
        // of accumulating a transcript.
        let (ws, _dir) = workspace();
        let id = AgentId::new();
        ws.write(id, "Manager", "first belief").unwrap();
        ws.write(id, "Manager", "corrected belief").unwrap();
        assert_eq!(ws.read(id), "corrected belief");
    }

    #[test]
    fn a_write_hands_back_the_version_it_replaced() {
        // The transcript shows the operator what an agent changed about itself,
        // and this is the only moment the version before it still exists.
        let (ws, _dir) = workspace();
        let id = AgentId::new();

        let first = ws.write(id, "Manager", "first belief").unwrap();
        assert_eq!(first.before, "", "an agent's first memory replaced nothing");

        let second = ws.write(id, "Manager", "corrected belief").unwrap();
        assert_eq!(second.before, "first belief");
    }

    #[test]
    fn the_replaced_version_survives_a_rename() {
        // Lookup is by id, and so is this: an agent renamed between two writes
        // must not read as one that has just written its memory for the first
        // time.
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
    fn renaming_an_agent_moves_its_notes_without_losing_them() {
        let (ws, _dir) = workspace();
        let id = AgentId::new();
        ws.write(id, "Manager", "remembered").unwrap();
        let stored = ws.write(id, "Coordinator", "remembered").unwrap();

        assert!(stored.path.file_name().unwrap().to_str().unwrap().starts_with("coordinator-"));
        assert_eq!(ws.read(id), "remembered");
        // And exactly one file exists: the old one was moved, not copied.
        assert_eq!(fs::read_dir(ws.root()).unwrap().count(), 1);
    }

    #[test]
    fn notes_are_found_by_id_even_if_the_filename_is_stale() {
        // Lookup by id is what makes a failed rename harmless.
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
        // Same name, which the app allows once one of them is deleted.
        ws.write(a, "Manager", "mine").unwrap();
        ws.write(b, "Manager", "also mine").unwrap();

        assert_eq!(ws.read(a), "mine");
        assert_eq!(ws.read(b), "also mine");
    }

    #[test]
    fn oversized_notes_are_cut_and_reported() {
        // Silently storing a truncated file would let an agent believe it had
        // recorded something it had not.
        let (ws, _dir) = workspace();
        let id = AgentId::new();
        let huge = "a line of notes\n".repeat(1_000);

        let stored = ws.write(id, "Manager", &huge).unwrap();
        assert!(stored.truncated);
        assert!(stored.characters <= MAX_NOTES);
        assert!(ws.read(id).chars().count() <= MAX_NOTES);
    }

    #[test]
    fn truncation_lands_on_a_line_boundary() {
        let (ws, _dir) = workspace();
        let id = AgentId::new();
        ws.write(id, "Manager", &"a line of notes\n".repeat(1_000)).unwrap();
        assert!(ws.read(id).ends_with("a line of notes"), "cut mid-sentence");
    }

    #[test]
    fn notes_within_the_cap_are_stored_whole() {
        let (ws, _dir) = workspace();
        let id = AgentId::new();
        let body = "x".repeat(MAX_NOTES);
        let stored = ws.write(id, "Manager", &body).unwrap();
        assert!(!stored.truncated);
        assert_eq!(ws.read(id).chars().count(), MAX_NOTES);
    }

    #[test]
    fn deleting_an_agent_removes_its_notes() {
        let (ws, _dir) = workspace();
        let id = AgentId::new();
        ws.write(id, "Manager", "gone soon").unwrap();
        ws.remove(id);
        assert_eq!(ws.read(id), "");
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
            "notes escaped the workspace: {}",
            stored.path.display()
        );
    }
}
