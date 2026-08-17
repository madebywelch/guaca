//! Where the bytes of an attachment actually live.
//!
//! One flat directory beside the agents' memories, addressed by the SHA-256 of
//! the contents. Content addressing is not cleverness here, it is the cheapest
//! answer to the thing this feature does constantly: the same document is sent
//! to three agents, forwarded once more, and re-attached next week, and every
//! one of those is the same bytes. Storing them once also makes the store
//! append-only, which means nothing that is being read can be rewritten
//! underneath a reader.
//!
//! Nothing is ever deleted. A transcript that survives an agent's deletion is a
//! promise this app already makes, and a file the transcript refers to has to
//! outlive the agent too. Reclaiming space means walking every message for live
//! digests, which is a job for the day somebody notices the directory.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::domain::attachment::{mime_for, Attachment, MAX_FILE_BYTES};

#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("{name} is {bytes} bytes, and the limit is {max}")]
    TooBig { name: String, bytes: u64, max: u64 },
    #[error("a file must have a name")]
    Unnamed,
    #[error("no file here with that content")]
    Unknown,
    #[error("could not use the file store at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

impl FileError {
    fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        FileError::Io { path: path.into(), source }
    }
}

/// The files every agent and the operator can refer to.
#[derive(Debug, Clone)]
pub struct FileStore {
    root: PathBuf,
}

impl FileStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Takes bytes in and returns what a message should carry.
    ///
    /// Writing is skipped when the digest is already stored: the contents are
    /// the address, so a second write of the same file could only produce the
    /// same file.
    pub fn put(&self, name: &str, bytes: &[u8]) -> Result<Attachment, FileError> {
        let name = clean_name(name).ok_or(FileError::Unnamed)?;
        let size = bytes.len() as u64;
        if size > MAX_FILE_BYTES {
            return Err(FileError::TooBig { name, bytes: size, max: MAX_FILE_BYTES });
        }

        let digest = format!("{:x}", Sha256::digest(bytes));
        let path = self.path(&digest);
        if !path.exists() {
            let parent = path.parent().expect("a stored file is never at the root");
            fs::create_dir_all(parent).map_err(|e| FileError::io(parent, e))?;
            // Written under a temporary name and renamed, so a reader can never
            // open a file that is still being written. Renaming within one
            // directory is atomic on every filesystem this app runs on.
            let pending = path.with_extension("writing");
            fs::write(&pending, bytes).map_err(|e| FileError::io(&pending, e))?;
            fs::rename(&pending, &path).map_err(|e| FileError::io(&path, e))?;
        }

        Ok(Attachment { digest, name: name.clone(), mime: mime_for(&name), bytes: size })
    }

    /// Reads a file in from somewhere on the operator's disk.
    pub fn take(&self, source: &Path) -> Result<Attachment, FileError> {
        let name = source
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string)
            .ok_or(FileError::Unnamed)?;
        // Checked before reading rather than after: the point of a limit is not
        // to load a gigabyte into memory and then object to it.
        let size = fs::metadata(source).map_err(|e| FileError::io(source, e))?.len();
        if size > MAX_FILE_BYTES {
            return Err(FileError::TooBig { name, bytes: size, max: MAX_FILE_BYTES });
        }
        let bytes = fs::read(source).map_err(|e| FileError::io(source, e))?;
        self.put(&name, &bytes)
    }

    pub fn read(&self, digest: &str) -> Result<Vec<u8>, FileError> {
        let path = self.path(digest);
        if !path.exists() {
            return Err(FileError::Unknown);
        }
        fs::read(&path).map_err(|e| FileError::io(&path, e))
    }

    /// The text of a file, for a prompt, cut at `limit` characters.
    ///
    /// Lossy on purpose. This is only ever called for a type that is text, and
    /// one invalid byte in a log file should cost that byte rather than the
    /// whole document.
    pub fn read_text(&self, digest: &str, limit: usize) -> Result<(String, bool), FileError> {
        let bytes = self.read(digest)?;
        let text = String::from_utf8_lossy(&bytes);
        match text.char_indices().nth(limit) {
            Some((at, _)) => Ok((text[..at].to_string(), true)),
            None => Ok((text.to_string(), false)),
        }
    }

    fn path(&self, digest: &str) -> PathBuf {
        // Two levels, because one directory with every file this app has ever
        // seen is slow to list and unpleasant to look at.
        let (prefix, rest) = digest.split_at(2.min(digest.len()));
        self.root.join(prefix).join(rest)
    }
}

/// The name as it will be shown and as it will land on a machine.
///
/// A name arrives from an operator's disk or from a model, so it is treated as
/// hostile: directory separators would let an attachment write outside the
/// inbox it is placed in, and a leading dot hides the file from the agent that
/// was just told it is there.
fn clean_name(name: &str) -> Option<String> {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default().trim();
    let cleaned: String = base
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || "._- ()".contains(c) { c } else { '_' })
        .collect();
    let cleaned = cleaned.trim_matches(|c: char| c == '.' || c.is_whitespace()).to_string();
    (!cleaned.is_empty()).then_some(cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (FileStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        (FileStore::new(dir.path().join("files")), dir)
    }

    #[test]
    fn a_stored_file_comes_back_byte_for_byte() {
        let (files, _dir) = store();
        let attachment = files.put("brief.pdf", b"%PDF-1.7 not really").unwrap();

        assert_eq!(attachment.name, "brief.pdf");
        assert_eq!(attachment.mime, "application/pdf");
        assert_eq!(attachment.bytes, 19);
        assert_eq!(files.read(&attachment.digest).unwrap(), b"%PDF-1.7 not really");
    }

    #[test]
    fn the_same_file_twice_is_stored_once() {
        // The common case, not an edge case: one document sent to three agents
        // and forwarded on is four references to one set of bytes.
        let (files, _dir) = store();
        let first = files.put("draft.docx", b"contents").unwrap();
        let again = files.put("copy-of-draft.docx", b"contents").unwrap();

        assert_eq!(first.digest, again.digest, "the contents are the address");
        assert_eq!(again.name, "copy-of-draft.docx", "but each reference keeps its own name");
        assert_eq!(files.read(&first.digest).unwrap(), b"contents");
    }

    #[test]
    fn a_file_over_the_limit_is_refused_by_name_and_size() {
        // The message reaches an operator or a model, so it has to say which
        // file and by how much.
        let (files, _dir) = store();
        let huge = vec![0u8; (MAX_FILE_BYTES + 1) as usize];
        let err = files.put("enormous.zip", &huge).unwrap_err();
        let said = err.to_string();
        assert!(said.contains("enormous.zip"), "{said}");
        assert!(said.contains(&MAX_FILE_BYTES.to_string()), "{said}");
    }

    #[test]
    fn a_name_cannot_escape_the_directory_it_will_be_written_into() {
        // Names come from operators' disks and from models, and this one is
        // placed on an agent's machine later.
        let (files, _dir) = store();
        assert_eq!(files.put("../../etc/passwd", b"x").unwrap().name, "passwd");
        assert_eq!(files.put(r"C:\Users\me\notes.txt", b"x").unwrap().name, "notes.txt");
        // The name is written into a shell command when a file is placed on a
        // machine, so nothing that means something to a shell survives it.
        let risky = files.put("plans; rm -rf $HOME `id`.md", b"x").unwrap().name;
        assert!(
            !risky.contains([';', '$', '`', '&', '|', '>', '<']),
            "a name that reaches a command line kept its metacharacters: {risky}"
        );
        assert!(risky.ends_with(".md"), "and it is still recognisable: {risky}");
        assert!(files.put("...", b"x").is_err(), "a name that is only dots is not a name");
        assert!(files.put("   ", b"x").is_err());
    }

    #[test]
    fn reading_something_that_was_never_stored_says_so_rather_than_panicking() {
        let (files, _dir) = store();
        assert!(matches!(files.read(&"a".repeat(64)), Err(FileError::Unknown)));
    }

    #[test]
    fn text_is_cut_at_the_limit_and_says_it_was_cut() {
        let (files, _dir) = store();
        let stored = files.put("long.txt", "abcdefghij".repeat(10).as_bytes()).unwrap();

        let (whole, trimmed) = files.read_text(&stored.digest, 1_000).unwrap();
        assert_eq!(whole.len(), 100);
        assert!(!trimmed);

        let (cut, trimmed) = files.read_text(&stored.digest, 10).unwrap();
        assert_eq!(cut, "abcdefghij");
        assert!(trimmed, "a prompt that silently loses the rest of a file is a lie");
    }

    #[test]
    fn a_file_that_is_not_quite_utf8_still_reads() {
        let (files, _dir) = store();
        let stored = files.put("log.txt", b"ok \xff\xfe done").unwrap();
        let (text, _) = files.read_text(&stored.digest, 1_000).unwrap();
        assert!(text.starts_with("ok "), "{text:?}");
        assert!(text.ends_with(" done"), "{text:?}");
    }

    #[test]
    fn a_half_written_file_is_never_visible_under_its_own_name() {
        // Readers address a file by its digest the moment a message carrying it
        // is delivered, which can be while the sender is still writing.
        let (files, _dir) = store();
        let stored = files.put("big.bin", &vec![7u8; 4096]).unwrap();
        let path = files.path(&stored.digest);
        assert!(path.exists());
        assert!(
            !path.with_extension("writing").exists(),
            "the temporary name has to be gone once the file is readable"
        );
    }
}
