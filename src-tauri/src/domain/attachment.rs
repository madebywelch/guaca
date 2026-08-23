//! A file traveling between the operator, the agents, and their machines.
//!
//! The bytes are not here and never travel in an envelope. A transcript is read
//! in bulk (forty messages into every prompt, hundreds into the activity view),
//! so a document inlined into a message would be dragged through both every
//! time anybody looked at a channel. What an envelope carries is this: enough
//! to name the file, decide how to hand it to a model, and find the one copy of
//! the bytes on disk.

use serde::{Deserialize, Serialize};

/// The largest file this app will take in. Bigger than any brief or proposal,
/// small enough that a copy per recipient is not a memory event.
pub const MAX_FILE_BYTES: u64 = 25 * 1024 * 1024;

/// One file, as a message refers to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    /// SHA-256 of the contents, and the only address the bytes have. Two agents
    /// sending the same document store it once.
    pub digest: String,
    /// What a person calls it. Never a path: where a file came from on the
    /// operator's disk is nobody else's business, and where it lands on an
    /// agent's machine is decided by the runtime.
    pub name: String,
    pub mime: String,
    pub bytes: u64,
}

impl Attachment {
    /// Whether a model can be shown this directly, as a picture.
    pub fn is_image(&self) -> bool {
        self.mime.starts_with("image/")
    }

    /// Whether this can be read into a prompt as text.
    ///
    /// By type rather than by sniffing the bytes: a file that is only mostly
    /// text arrives in a prompt as a screenful of replacement characters, and
    /// the agent has a machine that can open it properly.
    pub fn is_text(&self) -> bool {
        self.mime.starts_with("text/") || matches!(self.mime.as_str(), "application/json")
    }

    /// Human size, for a line a model or an operator reads.
    pub fn size(&self) -> String {
        const KB: u64 = 1024;
        const MB: u64 = 1024 * KB;
        match self.bytes {
            b if b >= MB => format!("{:.1} MB", b as f64 / MB as f64),
            b if b >= KB => format!("{} KB", b.div_ceil(KB)),
            b => format!("{b} bytes"),
        }
    }
}

/// The type of a file, from its name.
///
/// Extension only. Content sniffing would let a file claim to be text and
/// arrive in a prompt as binary, and the list below is exactly the set this app
/// does something different with: shows a picture, inlines a document, or hands
/// it to a machine that knows more file types than this ever will.
pub fn mime_for(name: &str) -> String {
    let extension =
        name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).unwrap_or_default();
    let mime = match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "txt" | "log" => "text/plain",
        "md" | "markdown" => "text/markdown",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "yaml" | "yml" => "text/yaml",
        "toml" => "text/toml",
        "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "sh" | "sql" | "css" => "text/plain",
        "pdf" => "application/pdf",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    };
    mime.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attachment(name: &str, bytes: u64) -> Attachment {
        Attachment { digest: "d".repeat(64), name: name.to_string(), mime: mime_for(name), bytes }
    }

    #[test]
    fn a_picture_is_shown_and_a_document_is_not() {
        assert!(attachment("screen.png", 1).is_image());
        assert!(attachment("photo.JPEG", 1).is_image());
        assert!(!attachment("brief.pdf", 1).is_image());
        assert!(!attachment("brief.pdf", 1).is_text());
    }

    #[test]
    fn the_files_worth_reading_into_a_prompt_are_the_ones_that_are_text() {
        for name in ["notes.md", "data.csv", "config.json", "main.rs", "readme.txt"] {
            assert!(attachment(name, 1).is_text(), "{name} should be readable as text");
        }
        for name in ["proposal.docx", "sheet.xlsx", "archive.zip", "photo.png"] {
            assert!(!attachment(name, 1).is_text(), "{name} is not text");
        }
    }

    #[test]
    fn an_unknown_extension_is_something_only_a_machine_can_open() {
        // Not text. A prompt full of replacement characters is worse than a
        // file the agent opens on its own computer.
        assert_eq!(mime_for("model.safetensors"), "application/octet-stream");
        assert_eq!(mime_for("noextension"), "application/octet-stream");
        assert!(!attachment("model.safetensors", 1).is_text());
    }

    #[test]
    fn sizes_read_the_way_a_person_says_them() {
        assert_eq!(attachment("a", 512).size(), "512 bytes");
        assert_eq!(attachment("a", 2048).size(), "2 KB");
        assert_eq!(attachment("a", 1024 * 1024 * 3 / 2).size(), "1.5 MB");
    }
}
