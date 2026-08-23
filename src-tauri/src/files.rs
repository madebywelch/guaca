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
    // Dropping a folder is a thing people do, and the reason it cannot work is
    // not something they should have to infer from a read error.
    #[error("{name} is a folder. Attach the files inside it instead")]
    IsFolder { name: String },
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
        let about = fs::metadata(source).map_err(|e| FileError::io(source, e))?;
        if about.is_dir() {
            return Err(FileError::IsFolder { name });
        }
        let size = about.len();
        if size > MAX_FILE_BYTES {
            return Err(FileError::TooBig { name, bytes: size, max: MAX_FILE_BYTES });
        }
        let bytes = fs::read(source).map_err(|e| FileError::io(source, e))?;
        self.put(&name, &bytes)
    }

    /// What a message should carry for a file that is already stored.
    ///
    /// Everything but "which bytes" and "what to call it" is worked out here
    /// rather than taken from the caller. The webview holds a reference to a
    /// file it was shown, not authority over what that file is, so the size
    /// comes off the disk and the type comes off the name.
    pub fn reference(&self, digest: &str, name: &str) -> Result<Attachment, FileError> {
        let name = clean_name(name).ok_or(FileError::Unnamed)?;
        let path = self.stored(digest).ok_or(FileError::Unknown)?;
        let bytes = fs::metadata(&path).map_err(|e| FileError::io(&path, e))?.len();
        Ok(Attachment {
            digest: digest.to_string(),
            name: name.clone(),
            mime: mime_for(&name),
            bytes,
        })
    }

    pub fn read(&self, digest: &str) -> Result<Vec<u8>, FileError> {
        let path = self.stored(digest).ok_or(FileError::Unknown)?;
        fs::read(&path).map_err(|e| FileError::io(&path, e))
    }

    /// Answers the webview's request for one stored file.
    ///
    /// `target` is the path of a preview URL, still percent-encoded:
    /// `{digest}/{name}`. The name only decides the type that goes back, and a
    /// caller that renames a file cannot turn it into a different one, because
    /// the bytes are addressed by their content and nothing else.
    ///
    /// Ranges are answered because a webview's own PDF viewer asks for them.
    pub fn serve(&self, target: &str, range: Option<&str>) -> Result<Served, FileError> {
        let target = decode(target);
        let (digest, name) = target.split_once('/').unwrap_or((target.as_str(), ""));
        let bytes = self.read(digest)?;
        let mime = mime_for(name);

        match parse_range(range.unwrap_or_default(), bytes.len() as u64) {
            Some((start, end)) => {
                let total = bytes.len();
                Ok(Served {
                    status: 206,
                    mime,
                    body: bytes[start as usize..=end as usize].to_vec(),
                    content_range: Some(format!("bytes {start}-{end}/{total}")),
                })
            }
            None => Ok(Served { status: 200, mime, body: bytes, content_range: None }),
        }
    }

    /// Writes a copy of a stored file where a person can get at it.
    ///
    /// Never overwrites. Saving the same document twice means the operator
    /// wants a second copy or has lost the first, and neither is a reason for
    /// this app to destroy a file it did not write.
    pub fn save_copy(&self, digest: &str, name: &str, into: &Path) -> Result<PathBuf, FileError> {
        let name = clean_name(name).ok_or(FileError::Unnamed)?;
        let bytes = self.read(digest)?;
        fs::create_dir_all(into).map_err(|e| FileError::io(into, e))?;
        let path = free_path(into, &name, digest);
        fs::write(&path, &bytes).map_err(|e| FileError::io(&path, e))?;
        Ok(path)
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

    /// Where a stored file is, or `None` when nothing is stored there.
    ///
    /// The gate on every read. A digest now arrives from the webview asking for
    /// a preview, and it is joined onto a directory: anything that is not the
    /// 64 hex characters of a SHA-256 is refused before that join rather than
    /// after it, so nothing that means something to a path ever reaches one.
    fn stored(&self, digest: &str) -> Option<PathBuf> {
        if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let path = self.path(digest);
        path.exists().then_some(path)
    }
}

/// One stored file, on its way to the webview.
///
/// A preview is bytes, and bytes do not travel over IPC in this app: the
/// webview reads a file over a URL scheme of its own instead, and this is what
/// that scheme hands back. `app.rs` turns it into an HTTP response.
#[derive(Debug, PartialEq)]
pub struct Served {
    pub status: u16,
    pub mime: String,
    /// The whole file, or the slice a range asked for.
    pub body: Vec<u8>,
    /// `content-range`, on a partial answer only.
    pub content_range: Option<String>,
}

/// The range a webview asks for: `bytes=first-last`, with either end implied.
///
/// Anything else, a range past the end included, is `None` and gets the whole
/// file. That is a legal answer to a range request and a better one than an
/// error, which a picture cannot show and a PDF viewer gives up on.
fn parse_range(header: &str, total: u64) -> Option<(u64, u64)> {
    if total == 0 {
        return None;
    }
    let spec = header.trim().strip_prefix("bytes=")?.split(',').next()?.trim();
    let (first, last) = spec.split_once('-')?;
    let (start, end) = match (first.trim(), last.trim()) {
        // A suffix range: the last n bytes, however many there turn out to be.
        ("", n) => (total.saturating_sub(n.parse::<u64>().ok()?), total - 1),
        (s, "") => (s.parse().ok()?, total - 1),
        (s, e) => (s.parse().ok()?, e.parse::<u64>().ok()?.min(total - 1)),
    };
    (start <= end && start < total).then_some((start, end))
}

/// Percent-decoding, for the one thing that arrives encoded: the name in a
/// preview URL, which holds spaces and brackets often enough to matter.
fn decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let pair = (at + 2 < bytes.len()).then(|| &raw[at + 1..at + 3]);
        match (bytes[at], pair.and_then(|p| u8::from_str_radix(p, 16).ok())) {
            (b'%', Some(byte)) => {
                out.push(byte);
                at += 3;
            }
            (byte, _) => {
                out.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `brief.pdf`, then `brief (2).pdf`, then `brief (3).pdf`.
///
/// The digest is the last resort rather than the first, because a person
/// looking in their downloads folder should see the name they know. Two files
/// that reach the same digest-suffixed name hold the same bytes by definition,
/// so the one write that can land on an existing file cannot lose anything.
fn free_path(dir: &Path, name: &str, digest: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, format!(".{extension}")),
        _ => (name, String::new()),
    };
    (2..100)
        .map(|n| dir.join(format!("{stem} ({n}){extension}")))
        .find(|candidate| !candidate.exists())
        .unwrap_or_else(|| {
            dir.join(format!("{stem} ({}){extension}", &digest[..8.min(digest.len())]))
        })
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
        assert!(risky.ends_with(".md"), "and it is still recognizable: {risky}");
        assert!(files.put("...", b"x").is_err(), "a name that is only dots is not a name");
        assert!(files.put("   ", b"x").is_err());
    }

    #[test]
    fn reading_something_that_was_never_stored_says_so_rather_than_panicking() {
        let (files, _dir) = store();
        assert!(matches!(files.read(&"a".repeat(64)), Err(FileError::Unknown)));
    }

    #[test]
    fn a_folder_is_refused_with_something_a_person_can_do_about_it() {
        // People drop folders. "Is a directory (os error 21)" is a true answer
        // and a useless one.
        let (files, dir) = store();
        let err = files.take(dir.path()).unwrap_err();
        assert!(err.to_string().contains("folder"), "{err}");
        assert!(err.to_string().contains("files inside"), "{err}");
    }

    #[test]
    fn a_preview_gets_the_bytes_and_the_type_the_name_implies() {
        let (files, _dir) = store();
        let stored = files.put("brief.pdf", b"%PDF-1.7 not really").unwrap();

        let served = files.serve(&format!("{}/brief.pdf", stored.digest), None).unwrap();
        assert_eq!(served.status, 200);
        assert_eq!(served.mime, "application/pdf");
        assert_eq!(served.body, b"%PDF-1.7 not really");
        assert_eq!(served.content_range, None);
    }

    #[test]
    fn a_name_with_spaces_arrives_encoded_and_is_still_the_same_file() {
        // The webview builds the URL with `encodeURIComponent`, so the
        // separator and every space in the name reach this side as escapes.
        let (files, _dir) = store();
        let stored = files.put("last quarter (final).pdf", b"x").unwrap();

        let served = files
            .serve(&format!("{}%2Flast%20quarter%20(final).pdf", stored.digest), None)
            .unwrap();
        assert_eq!(served.mime, "application/pdf");
        assert_eq!(served.body, b"x");
    }

    #[test]
    fn a_digest_that_is_not_a_digest_never_reaches_the_filesystem() {
        // This one arrives from the webview, and it is joined onto a path.
        let (files, dir) = store();
        std::fs::write(dir.path().join("secret"), b"private").unwrap();

        for target in
            ["../../secret/x.txt", "..%2F..%2Fsecret%2Fx.txt", "/etc/passwd/x.txt", "a/x.txt", ""]
        {
            assert!(
                matches!(files.serve(target, None), Err(FileError::Unknown)),
                "{target} was not refused"
            );
        }
    }

    #[test]
    fn a_range_gets_the_slice_it_asked_for_and_says_which_slice_it_is() {
        // A webview's own PDF viewer asks in ranges, and a viewer handed the
        // whole file for every range request re-reads it once per page.
        let (files, _dir) = store();
        let stored = files.put("doc.pdf", b"0123456789").unwrap();
        let target = format!("{}/doc.pdf", stored.digest);

        let served = files.serve(&target, Some("bytes=2-5")).unwrap();
        assert_eq!(served.status, 206);
        assert_eq!(served.body, b"2345");
        assert_eq!(served.content_range.as_deref(), Some("bytes 2-5/10"));

        let open_ended = files.serve(&target, Some("bytes=7-")).unwrap();
        assert_eq!(open_ended.body, b"789");
        assert_eq!(open_ended.content_range.as_deref(), Some("bytes 7-9/10"));

        // The tail, which is where a PDF keeps the table it opens with.
        let suffix = files.serve(&target, Some("bytes=-3")).unwrap();
        assert_eq!(suffix.body, b"789");
        assert_eq!(suffix.content_range.as_deref(), Some("bytes 7-9/10"));
    }

    #[test]
    fn a_range_that_makes_no_sense_gets_the_whole_file_rather_than_an_error() {
        // Ignoring a range is a legal answer and a picture can show it. An
        // error is a broken image icon.
        let (files, _dir) = store();
        let stored = files.put("doc.pdf", b"0123456789").unwrap();
        let target = format!("{}/doc.pdf", stored.digest);

        for header in ["bytes=50-60", "bytes=9-2", "items=0-1", "bytes=x-y", "", "bytes=-"] {
            let served = files.serve(&target, Some(header)).unwrap();
            assert_eq!(served.status, 200, "{header} should have been answered in full");
            assert_eq!(served.body.len(), 10, "{header}");
        }

        // Past the end but overlapping is still a slice, not a refusal.
        let clamped = files.serve(&target, Some("bytes=8-99")).unwrap();
        assert_eq!(clamped.body, b"89");
        assert_eq!(clamped.content_range.as_deref(), Some("bytes 8-9/10"));
    }

    #[test]
    fn a_reference_takes_the_bytes_and_the_name_and_works_out_the_rest() {
        // The webview holds a reference to a file it was shown, not authority
        // over what that file is.
        let (files, _dir) = store();
        let stored = files.put("sheet.csv", b"a,b,c").unwrap();

        let referenced = files.reference(&stored.digest, "../sheet.csv").unwrap();
        assert_eq!(referenced.name, "sheet.csv", "a name is cleaned wherever it comes from");
        assert_eq!(referenced.mime, "text/csv", "the type follows the name, not the caller");
        assert_eq!(referenced.bytes, 5, "and the size comes off the disk");

        assert!(matches!(files.reference(&"b".repeat(64), "ghost.txt"), Err(FileError::Unknown)));
    }

    #[test]
    fn a_saved_copy_keeps_its_name_and_never_overwrites_one_already_there() {
        let (files, dir) = store();
        let stored = files.put("brief.pdf", b"first").unwrap();
        let downloads = dir.path().join("Downloads");

        let first = files.save_copy(&stored.digest, "brief.pdf", &downloads).unwrap();
        assert_eq!(first.file_name().unwrap(), "brief.pdf");
        assert_eq!(std::fs::read(&first).unwrap(), b"first");

        let again = files.save_copy(&stored.digest, "brief.pdf", &downloads).unwrap();
        assert_eq!(again.file_name().unwrap(), "brief (2).pdf");
        assert!(first.exists(), "the first copy is still there");
    }

    #[test]
    fn saving_something_that_is_not_stored_says_so_before_it_writes_anything() {
        let (files, dir) = store();
        let downloads = dir.path().join("Downloads");
        assert!(matches!(
            files.save_copy(&"c".repeat(64), "ghost.pdf", &downloads),
            Err(FileError::Unknown)
        ));
        assert!(!downloads.join("ghost.pdf").exists());
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
