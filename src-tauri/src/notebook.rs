use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::domain::ids::AgentId;

pub const MAX_FILE: usize = 64_000;

pub const MAX_FILES: usize = 200;

pub const MAX_DEPTH: usize = 4;

pub const LISTED: usize = 40;

const EXTENSIONS: [&str; 4] = ["md", "txt", "json", "csv"];

#[derive(Debug, thiserror::Error)]
pub enum NotebookError {
    #[error(
        "`{given}` is not a notebook path: {why}. Use a relative path of plain names, such as \
         `leads/acme.md`"
    )]
    BadPath { given: String, why: &'static str },
    #[error("there is no `{0}` in your notebook. `list` shows what is there")]
    NotFound(String),
    #[error(
        "that would make `{path}` {chars} characters, and a notebook file holds {MAX_FILE}. Split \
         it, or delete what you no longer need from it"
    )]
    TooLong { path: String, chars: usize },
    #[error(
        "your notebook already holds {MAX_FILES} files. Delete ones you no longer need, or fold \
         several into one, before writing another"
    )]
    Full,
    #[error("`{0}` already exists. Delete it first, or move to another name")]
    Exists(String),
    #[error("could not access the notebook at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

fn io(path: &Path) -> impl FnOnce(io::Error) -> NotebookError + '_ {
    move |source| NotebookError::Io { path: path.to_path_buf(), source }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub path: String,
    pub chars: usize,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    pub path: String,
    pub chars: usize,
    pub created: bool,
}

pub fn clean_path(raw: &str) -> Result<String, NotebookError> {
    let given = raw.trim();
    let bad = |why| NotebookError::BadPath { given: given.to_string(), why };
    let trimmed = given.trim_start_matches("./").trim_start_matches('/');
    if trimmed.is_empty() {
        return Err(bad("it is empty"));
    }
    let parts: Vec<&str> = trimmed.split('/').collect();
    if parts.len() > MAX_DEPTH + 1 {
        return Err(bad("folders nest at most four deep"));
    }
    for part in &parts {
        if part.is_empty() || *part == "." || *part == ".." {
            return Err(bad("each part has to be a name, not `..` or empty"));
        }
        if part.starts_with('.') {
            return Err(bad("a name cannot start with a dot"));
        }
        if !part.bytes().all(|b| b.is_ascii_alphanumeric() || b"._- ".contains(&b)) {
            return Err(bad("names are letters, digits, spaces, dots, dashes and underscores"));
        }
    }
    let mut path = parts.join("/");
    let name = parts.last().copied().unwrap_or_default();
    match name.rsplit_once('.') {
        Some((_, ext)) if EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()) => {}
        Some(_) => return Err(bad("a file is .md, .txt, .json or .csv")),
        None => path.push_str(".md"),
    }
    Ok(path)
}

#[derive(Debug, Clone)]
pub struct Notebooks {
    root: PathBuf,
}

impl Notebooks {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn dir(&self, agent: AgentId) -> PathBuf {
        self.root.join(agent.to_string())
    }

    fn locate(&self, agent: AgentId, clean: &str) -> Result<PathBuf, NotebookError> {
        let dir = self.dir(agent);
        let mut at = dir.clone();
        for part in Path::new(clean).components() {
            let Component::Normal(name) = part else {
                return Err(NotebookError::BadPath {
                    given: clean.into(),
                    why: "it leaves the folder",
                });
            };
            at.push(name);
            if fs::symlink_metadata(&at).is_ok_and(|meta| meta.file_type().is_symlink()) {
                return Err(NotebookError::BadPath { given: clean.into(), why: "it is a link" });
            }
        }
        Ok(at)
    }

    pub fn list(&self, agent: AgentId) -> Vec<Entry> {
        let dir = self.dir(agent);
        let mut out = Vec::new();
        walk(&dir, &dir, 0, &mut out);
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }

    pub fn read(&self, agent: AgentId, raw: &str) -> Result<String, NotebookError> {
        let clean = clean_path(raw)?;
        let path = self.locate(agent, &clean)?;
        match fs::read_to_string(&path) {
            Ok(text) => Ok(text),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                Err(NotebookError::NotFound(clean))
            }
            Err(err) => Err(io(&path)(err)),
        }
    }

    pub fn write(
        &self,
        agent: AgentId,
        raw: &str,
        content: &str,
    ) -> Result<Written, NotebookError> {
        let clean = clean_path(raw)?;
        let body = content.trim_end().to_string() + "\n";
        self.store(agent, &clean, body)
    }

    pub fn append(
        &self,
        agent: AgentId,
        raw: &str,
        content: &str,
    ) -> Result<Written, NotebookError> {
        let clean = clean_path(raw)?;
        let before = match self.read(agent, &clean) {
            Ok(text) => text,
            Err(NotebookError::NotFound(_)) => String::new(),
            Err(err) => return Err(err),
        };
        let mut body = before.trim_end().to_string();
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str(content.trim_end());
        body.push('\n');
        self.store(agent, &clean, body)
    }

    fn store(&self, agent: AgentId, clean: &str, body: String) -> Result<Written, NotebookError> {
        let chars = body.chars().count();
        if chars > MAX_FILE {
            return Err(NotebookError::TooLong { path: clean.to_string(), chars });
        }
        let path = self.locate(agent, clean)?;
        let created = !path.exists();
        if created && self.list(agent).len() >= MAX_FILES {
            return Err(NotebookError::Full);
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io(parent))?;
        }
        let temp = path.with_extension("tmp-write");
        fs::write(&temp, &body).map_err(io(&temp))?;
        fs::rename(&temp, &path).map_err(io(&path))?;
        Ok(Written { path: clean.to_string(), chars, created })
    }

    pub fn rename(&self, agent: AgentId, from: &str, to: &str) -> Result<String, NotebookError> {
        let (from, to) = (clean_path(from)?, clean_path(to)?);
        let (source, target) = (self.locate(agent, &from)?, self.locate(agent, &to)?);
        if !source.is_file() {
            return Err(NotebookError::NotFound(from));
        }
        if target.exists() {
            return Err(NotebookError::Exists(to));
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(io(parent))?;
        }
        fs::rename(&source, &target).map_err(io(&target))?;
        self.prune(agent, source.parent());
        Ok(to)
    }

    pub fn delete(&self, agent: AgentId, raw: &str) -> Result<bool, NotebookError> {
        let clean = clean_path(raw)?;
        let path = self.locate(agent, &clean)?;
        match fs::remove_file(&path) {
            Ok(()) => {
                self.prune(agent, path.parent());
                Ok(true)
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(io(&path)(err)),
        }
    }

    fn prune(&self, agent: AgentId, mut folder: Option<&Path>) {
        let dir = self.dir(agent);
        while let Some(at) = folder.filter(|at| at.starts_with(&dir) && *at != dir) {
            if fs::remove_dir(at).is_err() {
                break;
            }
            folder = at.parent();
        }
    }

    pub fn remove_all(&self, agent: AgentId) -> Result<(), NotebookError> {
        let dir = self.dir(agent);
        match fs::remove_dir_all(&dir) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => Err(io(&dir)(err)),
            _ => Ok(()),
        }
    }
}

fn walk(root: &Path, at: &Path, depth: usize, out: &mut Vec<Entry>) {
    let Ok(entries) = fs::read_dir(at) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else { continue };
        let path = entry.path();
        if kind.is_dir() && depth < MAX_DEPTH {
            walk(root, &path, depth + 1, out);
        } else if kind.is_file() {
            let Ok(relative) = path.strip_prefix(root) else { continue };
            let relative = relative.to_string_lossy().replace('\\', "/");
            if clean_path(&relative).ok().as_deref() != Some(relative.as_str()) {
                continue;
            }
            let meta = entry.metadata().ok();
            let updated_at = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|since| since.as_millis() as i64)
                .unwrap_or(0);
            let chars = fs::read_to_string(&path).map(|text| text.chars().count()).unwrap_or(0);
            out.push(Entry { path: relative, chars, updated_at });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notebooks() -> (Notebooks, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        (Notebooks::new(dir.path().join("notebooks")), dir)
    }

    #[test]
    fn a_path_stays_inside_the_folder_whatever_a_model_types() {
        assert_eq!(clean_path("leads/acme").unwrap(), "leads/acme.md");
        assert_eq!(clean_path("/log.txt").unwrap(), "log.txt");
        assert_eq!(clean_path("./a/b.json").unwrap(), "a/b.json");
        for bad in
            ["", "../x.md", "a/../../x", ".hidden.md", "a//b.md", "x.sh", "a/b/c/d/e/f.md", "a\\b"]
        {
            assert!(matches!(clean_path(bad), Err(NotebookError::BadPath { .. })), "{bad}");
        }
        assert!(clean_path("x.sh").unwrap_err().to_string().contains("leads/acme.md"));
    }

    #[test]
    fn an_agent_shapes_its_own_folder_and_reads_back_what_it_wrote() {
        let (books, _dir) = notebooks();
        let me = AgentId::new();
        let first = books.write(me, "leads/acme", "# Acme\nWarm.").unwrap();
        assert!(first.created);
        books.append(me, "log", "- tried the API").unwrap();
        books.append(me, "log", "- it worked").unwrap();
        assert_eq!(books.read(me, "log.md").unwrap(), "- tried the API\n- it worked\n");
        assert_eq!(
            books.list(me).iter().map(|e| e.path.as_str()).collect::<Vec<_>>(),
            ["leads/acme.md", "log.md"]
        );
        assert!(!books.write(me, "leads/acme.md", "# Acme\nCold.").unwrap().created);

        assert_eq!(
            books.rename(me, "leads/acme.md", "archive/acme.md").unwrap(),
            "archive/acme.md"
        );
        assert!(!books.dir(me).join("leads").exists(), "an emptied folder goes with its file");
        assert!(matches!(
            books.rename(me, "log.md", "archive/acme.md"),
            Err(NotebookError::Exists(_))
        ));
        assert!(books.delete(me, "log").unwrap());
        assert!(!books.delete(me, "log").unwrap());
        assert!(matches!(books.read(me, "log"), Err(NotebookError::NotFound(_))));

        assert!(books.list(AgentId::new()).is_empty());
    }

    #[test]
    fn a_notebook_is_bounded_and_says_how() {
        let (books, _dir) = notebooks();
        let me = AgentId::new();
        let long = "w".repeat(MAX_FILE + 1);
        assert!(matches!(books.write(me, "big", &long), Err(NotebookError::TooLong { .. })));
        books.write(me, "log", &"w".repeat(MAX_FILE - 10)).unwrap();
        assert!(matches!(
            books.append(me, "log", &"x".repeat(20)),
            Err(NotebookError::TooLong { .. })
        ));
        for n in 0..MAX_FILES - 1 {
            books.write(me, &format!("f{n}"), "x").unwrap();
        }
        assert!(matches!(books.write(me, "one-more", "x"), Err(NotebookError::Full)));
        assert!(books.write(me, "f0", "rewritten").is_ok(), "a rewrite is not growth");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_somebody_left_is_not_a_way_out() {
        let (books, dir) = notebooks();
        let me = AgentId::new();
        books.write(me, "ok", "x").unwrap();
        std::os::unix::fs::symlink(dir.path(), books.dir(me).join("out")).unwrap();
        let err = books.write(me, "out/escape", "x").unwrap_err();
        assert!(matches!(err, NotebookError::BadPath { why: "it is a link", .. }), "{err}");
    }
}
