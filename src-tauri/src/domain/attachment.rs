use serde::{Deserialize, Serialize};

pub const MAX_FILE_BYTES: u64 = 25 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub digest: String,
    pub name: String,
    pub mime: String,
    pub bytes: u64,
}

impl Attachment {
    pub fn is_image(&self) -> bool {
        self.mime.starts_with("image/")
    }

    pub fn is_text(&self) -> bool {
        self.mime.starts_with("text/") || matches!(self.mime.as_str(), "application/json")
    }

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
