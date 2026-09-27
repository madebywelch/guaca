//! An agent's terminal on disk: its directory, and the three file tools that
//! work in it.
//!
//! The shell is [`crate::shell`] and the coding harnesses are
//! [`crate::coding`]; this is where both start, and what `read`, `write` and
//! `edit` mean. Each agent given a terminal has one directory under the app's
//! data, named for its id so a rename moves nothing, made the first time it is
//! needed and kept until the agent is purged. Taking the terminal away keeps
//! it: being allowed a terminal is a decision, and the work in the directory
//! outlives any one of them.
//!
//! ## Why three file tools beside a shell
//!
//! The shell can read and write files, and a model editing through it writes
//! `sed` expressions and heredocs that break on the first quote or dollar sign
//! in the text. Every coding harness ships an exact-replacement edit for that
//! reason, and this is the same tool: the model names the text it wants gone
//! and the text it wants instead, and either that text is in the file exactly
//! once or nothing changes. `read` is bounded so a large file comes back in
//! pages rather than as one tool result the size of a context window.
//!
//! ## Where they reach, and why that is not a boundary
//!
//! `read` takes a path relative to the agent's directory or an absolute one
//! anywhere, because reading changes nothing and the shell can read anywhere
//! anyway. `write` and `edit` stay inside the directory, resolved through
//! links, because the ordinary case of an agent editing a file outside the
//! directory it was given is a mistake about where it is standing, and the
//! refusal says so. Neither is confinement: the shell can write anywhere its
//! user can, and nothing here may ever be described as a sandbox.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::domain::ids::AgentId;

/// How many lines one `read` returns by default, and at most.
pub const READ_LINES: usize = 2_000;

/// How many bytes one `read` returns at most, whatever the line count.
pub const READ_BYTES: usize = 50 * 1024;

/// How much of a file is looked at to decide whether it is text.
const SNIFF: usize = 8 * 1024;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum TerminalError {
    #[error("there is no file at `{0}`. `shell` with `ls` shows what is there")]
    NotFound(String),
    #[error(
        "`{0}` is outside your terminal's directory. `write`, `edit` and `code` work inside it; \
         clone or copy what you need into it first"
    )]
    Outside(String),
    #[error("`{0}` is a directory, not a file. `shell` with `ls` shows what is in it")]
    IsDirectory(String),
    #[error("`{0}` is not a directory in your terminal. `shell` with `ls` shows what is there")]
    NotADirectory(String),
    #[error(
        "`{0}` is not text. Use `shell` to look at it (`file`, `xxd | head`), or `attach_file` \
         to hand it over"
    )]
    NotText(String),
    #[error("{0}")]
    BadEdit(String),
    #[error("`{path}` could not be {doing}: {reason}")]
    Io { path: String, doing: &'static str, reason: String },
}

fn failed<'a>(path: &'a Path, doing: &'static str) -> impl FnOnce(io::Error) -> TerminalError + 'a {
    move |err| match err.kind() {
        io::ErrorKind::NotFound => TerminalError::NotFound(path.display().to_string()),
        _ => TerminalError::Io { path: path.display().to_string(), doing, reason: err.to_string() },
    }
}

/// One replacement an `edit` makes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct Replacement {
    pub old_text: String,
    pub new_text: String,
}

/// A page of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// As the model named it, or relative to the directory when it is inside.
    pub path: String,
    /// The first line returned, counting from one.
    pub first: usize,
    /// The last line returned. Less than `first` when the offset is past the
    /// end.
    pub last: usize,
    pub total: usize,
    pub text: String,
}

impl Page {
    /// The page as a tool result: where it is in the file, then the text, then
    /// how to get the rest.
    pub fn render(&self) -> String {
        if self.total == 0 {
            return format!("{} is empty.", self.path);
        }
        if self.last < self.first {
            return format!(
                "{} has {} lines, so there is nothing from line {} on.",
                self.path, self.total, self.first
            );
        }
        let tail = if self.last < self.total {
            format!("[More remains. Call read with offset {} to continue.]", self.last + 1)
        } else {
            "[End of file.]".to_string()
        };
        format!(
            "{}, lines {}-{} of {}:\n\n{}\n\n{tail}",
            self.path, self.first, self.last, self.total, self.text
        )
    }
}

#[derive(Debug, Clone)]
pub struct Terminals {
    root: PathBuf,
}

impl Terminals {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Where an agent's directory is, whether or not it exists yet.
    pub fn dir(&self, agent: AgentId) -> PathBuf {
        self.root.join(agent.to_string())
    }

    /// The directory, made if it is not there, as the path every tool and
    /// every harness is started in.
    ///
    /// Canonical, because a harness reports the paths it touched in its own
    /// spelling of the directory, and on macOS the data directory can be
    /// reached through a link.
    pub fn ensure(&self, agent: AgentId) -> Result<PathBuf, TerminalError> {
        let dir = self.dir(agent);
        fs::create_dir_all(&dir).map_err(failed(&dir, "made"))?;
        fs::canonicalize(&dir).map_err(failed(&dir, "opened"))
    }

    /// Everything in it, when the agent is gone for good.
    pub fn remove_all(&self, agent: AgentId) {
        let dir = self.dir(agent);
        if let Err(err) = fs::remove_dir_all(&dir) {
            if err.kind() != io::ErrorKind::NotFound {
                tracing::warn!(%err, dir = %dir.display(), "could not remove a terminal");
            }
        }
    }

    /// A directory inside the terminal, for a coding job to start in. Nothing
    /// or `.` is the terminal itself.
    ///
    /// Returns the absolute path and the path relative to the terminal, which
    /// is how a job's directory is written down and shown.
    pub fn directory(
        &self,
        agent: AgentId,
        raw: Option<&str>,
    ) -> Result<(PathBuf, String), TerminalError> {
        let home = self.ensure(agent)?;
        let given = raw.map(str::trim).filter(|raw| !raw.is_empty()).unwrap_or(".");
        let at = fs::canonicalize(joined(&home, given))
            .map_err(|_| TerminalError::NotADirectory(given.to_string()))?;
        if !at.starts_with(&home) {
            return Err(TerminalError::Outside(given.to_string()));
        }
        if !at.is_dir() {
            return Err(TerminalError::NotADirectory(given.to_string()));
        }
        let relative = at.strip_prefix(&home).unwrap_or(Path::new("")).to_string_lossy();
        let relative = if relative.is_empty() { ".".to_string() } else { relative.into_owned() };
        Ok((at, relative))
    }

    /// A page of a text file.
    ///
    /// `offset` is the first line wanted, counting from one, and `limit` how
    /// many; both are clamped rather than refused, because a model asking for
    /// line zero or for ten thousand lines wants the start of the file or as
    /// much as there is, and a refusal would only be retried.
    pub fn read(
        &self,
        agent: AgentId,
        raw: &str,
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> Result<Page, TerminalError> {
        let home = self.ensure(agent)?;
        let path = joined(&home, raw.trim());
        if path.is_dir() {
            return Err(TerminalError::IsDirectory(raw.trim().to_string()));
        }
        let bytes = fs::read(&path).map_err(failed(&path, "read"))?;
        let text = text_of(&bytes).ok_or_else(|| TerminalError::NotText(raw.trim().into()))?;

        let lines: Vec<&str> = text.lines().collect();
        let first = offset.unwrap_or(1).max(1);
        let limit = limit.unwrap_or(READ_LINES).clamp(1, READ_LINES);
        let mut kept = String::new();
        let mut last = first - 1;
        for line in lines.iter().skip(first - 1).take(limit) {
            if !kept.is_empty() && kept.len() + line.len() + 1 > READ_BYTES {
                break;
            }
            if !kept.is_empty() {
                kept.push('\n');
            }
            kept.push_str(line);
            last += 1;
        }
        Ok(Page { path: shown(&home, &path, raw), first, last, total: lines.len(), text: kept })
    }

    /// Creates or replaces a file inside the directory, making the folders it
    /// needs. Answers with what happened, in words.
    pub fn write(&self, agent: AgentId, raw: &str, content: &str) -> Result<String, TerminalError> {
        let home = self.ensure(agent)?;
        let path = inside(&home, raw.trim())?;
        if path.is_dir() {
            return Err(TerminalError::IsDirectory(raw.trim().to_string()));
        }
        let created = !path.exists();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(failed(parent, "made"))?;
        }
        replace(&path, content)?;
        let lines = content.lines().count();
        Ok(format!(
            "{} {}: {} line{}, {} bytes.",
            if created { "Created" } else { "Replaced" },
            shown(&home, &path, raw),
            lines,
            if lines == 1 { "" } else { "s" },
            content.len()
        ))
    }

    /// Replaces text in a file inside the directory.
    ///
    /// Every replacement is matched against the file as it was before any of
    /// them, each has to match exactly once, and none may overlap another. If
    /// any of that is not so, nothing is written: half an edit is a file
    /// nobody asked for.
    pub fn edit(
        &self,
        agent: AgentId,
        raw: &str,
        edits: &[Replacement],
    ) -> Result<String, TerminalError> {
        let home = self.ensure(agent)?;
        let path = inside(&home, raw.trim())?;
        let name = shown(&home, &path, raw);
        if path.is_dir() {
            return Err(TerminalError::IsDirectory(name));
        }
        let bytes = fs::read(&path).map_err(failed(&path, "read"))?;
        let original = text_of(&bytes).ok_or_else(|| TerminalError::NotText(name.clone()))?;
        let edited = apply(&original, edits).map_err(TerminalError::BadEdit)?;
        replace(&path, &edited)?;
        Ok(format!(
            "Edited {name}: {} replacement{}.",
            edits.len(),
            if edits.len() == 1 { "" } else { "s" }
        ))
    }
}

/// The edits applied to one text, or why not, worded for the model.
fn apply(original: &str, edits: &[Replacement]) -> Result<String, String> {
    if edits.is_empty() {
        return Err("give at least one edit: the exact old_text to replace, and new_text".into());
    }
    let mut spans: Vec<(usize, usize, &str)> = Vec::with_capacity(edits.len());
    for (n, edit) in edits.iter().enumerate() {
        let which = if edits.len() == 1 { String::new() } else { format!("edit {}: ", n + 1) };
        if edit.old_text.is_empty() {
            return Err(format!(
                "{which}old_text is empty. Name the exact text to replace; to write a whole \
                 file, use write"
            ));
        }
        if edit.old_text == edit.new_text {
            return Err(format!("{which}old_text and new_text are the same, so nothing changes"));
        }
        let mut found = original.match_indices(edit.old_text.as_str());
        let Some((at, _)) = found.next() else {
            return Err(format!(
                "{which}old_text is not in the file. It has to match exactly, including spaces \
                 and indentation. Read the file again and copy the text from it"
            ));
        };
        let more = found.count();
        if more > 0 {
            return Err(format!(
                "{which}old_text appears {} times. Include enough of the surrounding lines to \
                 match exactly one of them",
                more + 1
            ));
        }
        spans.push((at, at + edit.old_text.len(), edit.new_text.as_str()));
    }
    spans.sort_by_key(|span| span.0);
    if spans.windows(2).any(|pair| pair[1].0 < pair[0].1) {
        return Err("two edits overlap. Merge them into one edit".into());
    }
    let mut out = String::with_capacity(original.len());
    let mut from = 0;
    for (start, end, new_text) in spans {
        out.push_str(&original[from..start]);
        out.push_str(new_text);
        from = end;
    }
    out.push_str(&original[from..]);
    Ok(out)
}

/// A path the model gave, against the directory when it is relative, with
/// `.` and `..` taken out without touching the disk.
///
/// `~` is the user's home, as the shell would read it, and not the terminal:
/// a model that writes `~/.gitconfig` means that file, and quietly reading a
/// different one would answer a question it did not ask.
fn joined(home: &Path, raw: &str) -> PathBuf {
    let user = std::env::var_os("HOME").map(PathBuf::from);
    let raw = match (raw.strip_prefix('~'), user) {
        (Some(rest), Some(user)) if rest.is_empty() || rest.starts_with('/') => {
            user.join(rest.trim_start_matches('/')).to_string_lossy().into_owned()
        }
        _ => raw.to_string(),
    };
    let given = Path::new(&raw);
    let start = if given.is_absolute() { PathBuf::new() } else { home.to_path_buf() };
    let mut out = start;
    for part in given.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The same, refused unless it lands inside the directory once every link on
/// the way has been followed.
///
/// The nearest part of the path that exists is what gets resolved, because a
/// file being created does not exist yet and a folder it is created in may not
/// either.
fn inside(home: &Path, raw: &str) -> Result<PathBuf, TerminalError> {
    let path = joined(home, raw);
    let mut existing = path.as_path();
    let mut rest = Vec::new();
    while fs::symlink_metadata(existing).is_err() {
        let Some(name) = existing.file_name() else { break };
        rest.push(name.to_os_string());
        let Some(parent) = existing.parent() else { break };
        existing = parent;
    }
    let mut real = fs::canonicalize(existing).map_err(|_| TerminalError::Outside(raw.into()))?;
    if !real.starts_with(home) {
        return Err(TerminalError::Outside(raw.into()));
    }
    for name in rest.into_iter().rev() {
        real.push(name);
    }
    Ok(real)
}

/// How a path is written back to the model: relative when it is inside the
/// directory, as given otherwise.
fn shown(home: &Path, path: &Path, raw: &str) -> String {
    let real = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    match real.strip_prefix(home) {
        Ok(relative) if !relative.as_os_str().is_empty() => relative.to_string_lossy().into(),
        _ => raw.trim().to_string(),
    }
}

/// The bytes as text, or `None` when they are not.
///
/// A NUL in the first few kilobytes is what `git` and `grep` both take to mean
/// binary, and it is the right test here for the same reason: a source file
/// never has one and almost every binary format does.
fn text_of(bytes: &[u8]) -> Option<String> {
    if bytes[..bytes.len().min(SNIFF)].contains(&0) {
        return None;
    }
    String::from_utf8(bytes.to_vec()).ok()
}

/// Writes through a temporary file beside the target and a rename, so a file
/// is either the old text or the new one and never half of each.
fn replace(path: &Path, content: &str) -> Result<(), TerminalError> {
    let temp = path.with_file_name(format!(
        ".{}.guaca-write",
        path.file_name().map(|name| name.to_string_lossy()).unwrap_or_default()
    ));
    fs::write(&temp, content).map_err(failed(&temp, "written"))?;
    // Keep the file's own permissions, or an edited script stops being
    // executable.
    if let Ok(meta) = fs::metadata(path) {
        let _ = fs::set_permissions(&temp, meta.permissions());
    }
    fs::rename(&temp, path).map_err(failed(path, "written"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terminal() -> (Terminals, AgentId, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        (Terminals::new(dir.path().join("terminals")), AgentId::new(), dir)
    }

    fn edit(old: &str, new: &str) -> Replacement {
        Replacement { old_text: old.into(), new_text: new.into() }
    }

    #[test]
    fn an_edit_that_does_not_match_exactly_once_changes_nothing() {
        let (terminals, agent, _dir) = terminal();
        terminals.write(agent, "a.txt", "one\ntwo\ntwo\n").unwrap();

        let missing = terminals.edit(agent, "a.txt", &[edit("three", "3")]).unwrap_err();
        assert!(missing.to_string().contains("Read the file again"), "{missing}");
        let twice = terminals.edit(agent, "a.txt", &[edit("two", "2")]).unwrap_err();
        assert!(twice.to_string().contains("appears 2 times"), "{twice}");
        // One good edit beside a bad one writes neither.
        let mixed = terminals.edit(agent, "a.txt", &[edit("one", "1"), edit("nope", "x")]);
        assert!(mixed.unwrap_err().to_string().starts_with("edit 2:"));
        let overlapping =
            terminals.edit(agent, "a.txt", &[edit("one\ntwo", "x"), edit("two\ntwo", "y")]);
        assert!(overlapping.unwrap_err().to_string().contains("overlap"));

        assert_eq!(
            fs::read_to_string(terminals.dir(agent).join("a.txt")).unwrap(),
            "one\ntwo\ntwo\n"
        );
    }

    #[test]
    fn edits_are_matched_against_the_file_as_it_was() {
        let (terminals, agent, _dir) = terminal();
        terminals.write(agent, "a.rs", "let a = 1;\nlet b = 2;\n").unwrap();
        let said = terminals
            .edit(
                agent,
                "a.rs",
                &[edit("let b = 2;", "let b = 3;"), edit("let a = 1;", "let a = 0;")],
            )
            .unwrap();
        assert_eq!(said, "Edited a.rs: 2 replacements.");
        assert_eq!(
            fs::read_to_string(terminals.dir(agent).join("a.rs")).unwrap(),
            "let a = 0;\nlet b = 3;\n"
        );
    }

    #[test]
    fn a_write_or_edit_outside_the_directory_is_refused_even_through_a_link() {
        let (terminals, agent, dir) = terminal();
        let home = terminals.ensure(agent).unwrap();
        let elsewhere = dir.path().join("elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();
        fs::write(elsewhere.join("x.txt"), "x").unwrap();

        for raw in
            ["../escape.txt", "/etc/guaca-test.txt", elsewhere.join("x.txt").to_str().unwrap()]
        {
            let refused = terminals.write(agent, raw, "no").unwrap_err();
            assert!(matches!(refused, TerminalError::Outside(_)), "{raw}: {refused}");
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&elsewhere, home.join("link")).unwrap();
            let through = terminals.write(agent, "link/x.txt", "no").unwrap_err();
            assert!(matches!(through, TerminalError::Outside(_)), "{through}");
            let edited = terminals.edit(agent, "link/x.txt", &[edit("x", "y")]).unwrap_err();
            assert!(matches!(edited, TerminalError::Outside(_)), "{edited}");
        }
        assert_eq!(fs::read_to_string(elsewhere.join("x.txt")).unwrap(), "x");
    }

    #[test]
    fn a_write_makes_the_folders_it_needs_and_says_what_it_did() {
        let (terminals, agent, _dir) = terminal();
        let said = terminals.write(agent, "repo/src/new.rs", "fn main() {}\n").unwrap();
        assert_eq!(said, "Created repo/src/new.rs: 1 line, 13 bytes.");
        let again = terminals.write(agent, "./repo/src/new.rs", "a\nb\n").unwrap();
        assert!(again.starts_with("Replaced repo/src/new.rs: 2 lines"), "{again}");
    }

    #[test]
    fn a_long_file_is_read_a_page_at_a_time() {
        let (terminals, agent, _dir) = terminal();
        let body: String = (1..=5000).map(|n| format!("line {n}\n")).collect();
        terminals.write(agent, "long.txt", &body).unwrap();

        let first = terminals.read(agent, "long.txt", None, None).unwrap();
        assert_eq!((first.first, first.last, first.total), (1, READ_LINES, 5000));
        assert!(first.render().contains("Call read with offset 2001"), "{}", first.render());

        let last = terminals.read(agent, "long.txt", Some(4999), Some(10)).unwrap();
        assert_eq!(last.text, "line 4999\nline 5000");
        assert!(last.render().ends_with("[End of file.]"));

        let past = terminals.read(agent, "long.txt", Some(9000), None).unwrap();
        assert!(past.render().contains("nothing from line 9000"), "{}", past.render());
    }

    #[test]
    fn a_page_stops_at_its_byte_limit_rather_than_its_line_count() {
        let (terminals, agent, _dir) = terminal();
        let wide = "x".repeat(1000);
        let body: String = (0..200).map(|_| format!("{wide}\n")).collect();
        terminals.write(agent, "wide.txt", &body).unwrap();
        let page = terminals.read(agent, "wide.txt", None, None).unwrap();
        assert!(page.text.len() <= READ_BYTES);
        assert!(page.last < 200 && page.last > 0, "{}", page.last);
    }

    #[test]
    fn a_binary_file_is_refused_with_a_way_to_look_at_it() {
        let (terminals, agent, _dir) = terminal();
        let home = terminals.ensure(agent).unwrap();
        fs::write(home.join("blob.bin"), [0u8, 1, 2, 3]).unwrap();
        let refused = terminals.read(agent, "blob.bin", None, None).unwrap_err();
        assert!(refused.to_string().contains("attach_file"), "{refused}");
    }

    #[test]
    fn read_reaches_an_absolute_path_and_names_a_missing_file() {
        let (terminals, agent, dir) = terminal();
        let outside = dir.path().join("readme.txt");
        fs::write(&outside, "hello\n").unwrap();
        let page = terminals.read(agent, outside.to_str().unwrap(), None, None).unwrap();
        assert_eq!(page.text, "hello");

        let missing = terminals.read(agent, "nope.txt", None, None).unwrap_err();
        assert!(matches!(missing, TerminalError::NotFound(_)), "{missing}");
    }

    #[test]
    fn a_job_directory_has_to_be_a_directory_inside_the_terminal() {
        let (terminals, agent, dir) = terminal();
        let home = terminals.ensure(agent).unwrap();
        fs::create_dir_all(home.join("guaca/src")).unwrap();
        fs::write(home.join("file.txt"), "x").unwrap();

        assert_eq!(terminals.directory(agent, None).unwrap().1, ".");
        assert_eq!(terminals.directory(agent, Some("guaca/")).unwrap().1, "guaca");
        assert!(matches!(
            terminals.directory(agent, Some("file.txt")),
            Err(TerminalError::NotADirectory(_))
        ));
        assert!(matches!(
            terminals.directory(agent, Some("missing")),
            Err(TerminalError::NotADirectory(_))
        ));
        let outside = dir.path().to_string_lossy().to_string();
        assert!(matches!(
            terminals.directory(agent, Some(&outside)),
            Err(TerminalError::Outside(_))
        ));
    }

    #[test]
    fn an_edited_script_stays_executable() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let (terminals, agent, _dir) = terminal();
            terminals.write(agent, "run.sh", "echo one\n").unwrap();
            let path = terminals.dir(agent).join("run.sh");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
            terminals.edit(agent, "run.sh", &[edit("one", "two")]).unwrap();
            assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o755);
        }
    }
}
