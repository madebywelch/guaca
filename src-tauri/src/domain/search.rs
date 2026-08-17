//! What a search of the workspace turns up.
//!
//! Only the half of the workspace that lives in SQLite. Agents and groups are
//! matched in the webview, which is already holding them for the rail, and
//! actions are not stored anywhere at all. What the webview does not hold is
//! the transcript: reading it into the renderer to search it there would copy
//! the database across IPC on every keystroke, so the transcript is searched
//! where it sits.
//!
//! Files and links are the same rows as the messages, read differently. A file
//! is a part of a message and a link is inside one's text, so both are found by
//! the scan that finds the message and are pulled out here rather than stored
//! anywhere of their own.

use serde::Serialize;

use super::attachment::Attachment;
use super::envelope::Participant;
use super::ids::{AgentId, MessageId};
use super::routine::Routine;

/// Everything the store has to say about one query.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHits {
    pub messages: Vec<MessageHit>,
    pub files: Vec<FileHit>,
    pub links: Vec<LinkHit>,
    pub routines: Vec<Routine>,
}

/// One message that matched, with enough to draw a row and open it.
///
/// Carries an excerpt rather than the body: a result list is forty rows of one
/// line each, and a search that returns whole transcripts to render forty
/// single lines has shipped the same bytes the query was meant to avoid.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageHit {
    pub id: MessageId,
    /// The channel to open to read it in context.
    pub channel_id: AgentId,
    pub from: Participant,
    pub to: Participant,
    pub excerpt: String,
    pub created_at: i64,
}

/// A file somebody attached, and where it was attached.
///
/// Deduplicated by digest: the bytes are addressed by content, so the same
/// document sent to three agents is one file and belongs on one row.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileHit {
    pub file: Attachment,
    pub message_id: MessageId,
    pub channel_id: AgentId,
    /// Who attached it, so the operator knows whose copy this was.
    pub from: Participant,
    pub created_at: i64,
}

/// A URL somebody wrote, and the message it was written in.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkHit {
    pub url: String,
    pub message_id: MessageId,
    pub channel_id: AgentId,
    pub created_at: i64,
}

/// A `LIKE` pattern matching the query as a literal substring.
///
/// Escaped, because `%` and `_` are wildcards: an operator searching for "50%"
/// would otherwise match every row in the table and be told it was a hit.
/// Case folding is left to SQLite's `LIKE`, which is insensitive for ASCII.
pub fn like_pattern(query: &str) -> String {
    let mut out = String::with_capacity(query.len() + 2);
    out.push('%');
    for ch in query.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('%');
    out
}

/// Whether a haystack contains a needle, ignoring case.
///
/// Used where the match has already been made in SQL and has to be made again
/// on a piece of the row: a URL out of a message's text, for instance.
pub fn contains_fold(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

/// A window of text around the first match, on one line.
///
/// Whitespace is collapsed first: a message body has paragraphs, and a row in a
/// result list has a line. An excerpt that starts partway into the body says so
/// with an ellipsis, so a match near the end does not read as the message
/// beginning there.
pub fn excerpt(text: &str, needle: &str, width: usize) -> String {
    let flat: Vec<char> = {
        let mut out: Vec<char> = Vec::new();
        let mut spacing = false;
        for ch in text.chars() {
            if ch.is_whitespace() {
                spacing = true;
                continue;
            }
            if spacing && !out.is_empty() {
                out.push(' ');
            }
            spacing = false;
            out.push(ch);
        }
        out
    };

    if flat.len() <= width {
        return flat.into_iter().collect();
    }

    // Counted in characters rather than bytes throughout: an excerpt cut at a
    // byte offset splits a multi-byte character, and the panic lands in the
    // middle of a search rather than anywhere near the message that caused it.
    let lower: Vec<char> = flat.iter().flat_map(|c| c.to_lowercase()).collect();
    let wanted: Vec<char> = needle.to_lowercase().chars().collect();
    let at = if wanted.is_empty() || lower.len() != flat.len() {
        // A handful of characters lowercase into two, and once the two lengths
        // disagree a position in `lower` is no longer a position in `flat`.
        // Showing the opening of the message beats showing the wrong part of it.
        None
    } else {
        lower.windows(wanted.len()).position(|w| w == wanted.as_slice())
    };

    let found = at.unwrap_or(0);
    let lead = width / 3;
    let start = found.saturating_sub(lead).min(flat.len().saturating_sub(width));
    let end = (start + width).min(flat.len());

    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.extend(&flat[start..end]);
    if end < flat.len() {
        out.push('…');
    }
    out
}

/// Every URL in a piece of text.
///
/// A scan rather than a parse. Message bodies are Markdown written by a model,
/// so a link arrives bare, inside `[title](url)`, or wrapped in angle brackets,
/// and all three end at the first character that cannot be in a URL.
pub fn links_in(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut cursor = 0;

    while cursor < text.len() {
        let Some(offset) = text[cursor..].find("http") else { break };
        let start = cursor + offset;
        let tail = &text[start..];
        if !(tail.starts_with("http://") || tail.starts_with("https://")) {
            cursor = start + "http".len();
            continue;
        }

        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '`' | '\\'))
            .map(|n| start + n)
            .unwrap_or(text.len());

        let url = trim_url(&text[start..end]);
        // A scheme and nothing after it is not a link, it is the word "https".
        if url.len() > "https://".len() {
            out.push(url);
        }
        cursor = end.max(start + 1);
    }

    out
}

/// Strips what a sentence put after a URL rather than inside it.
fn trim_url(raw: &str) -> &str {
    const TAIL: [char; 6] = ['.', ',', ';', ':', '!', '?'];
    let mut url = raw.trim_end_matches(TAIL);

    // A closing bracket belongs to the URL only when the URL opened it.
    // Markdown wraps every link in parentheses and Wikipedia puts them in the
    // path, so counting is the only rule that gets both right.
    loop {
        let unbalanced = (url.ends_with(')')
            && url.matches('(').count() < url.matches(')').count())
            || (url.ends_with(']') && url.matches('[').count() < url.matches(']').count());
        if !unbalanced {
            return url;
        }
        url = url[..url.len() - 1].trim_end_matches(TAIL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wildcard_in_the_query_is_matched_as_itself() {
        // Unescaped, this pattern matches every row in the table, and a search
        // for "50%" that returns the whole database reads as a broken index.
        assert_eq!(like_pattern("50%"), "%50\\%%");
        assert_eq!(like_pattern("a_b"), "%a\\_b%");
        assert_eq!(like_pattern("c:\\tmp"), "%c:\\\\tmp%");
    }

    #[test]
    fn an_empty_query_matches_everything() {
        assert_eq!(like_pattern(""), "%%");
        assert!(contains_fold("anything at all", ""));
    }

    #[test]
    fn matching_ignores_case() {
        assert!(contains_fold("The Quarterly Report", "quarterly"));
        assert!(!contains_fold("The Quarterly Report", "monthly"));
    }

    #[test]
    fn an_excerpt_is_one_line_with_the_match_in_it() {
        let body = "Line one.\n\nThe budget is signed off.\n\tLine three.";
        let out = excerpt(body, "budget", 24);
        assert!(out.contains("budget"), "the match has to survive the window: {out}");
        assert!(!out.contains('\n'), "a row is one line: {out}");
        assert!(!out.contains('\t'));
    }

    #[test]
    fn a_short_message_is_shown_whole_and_unmarked() {
        let out = excerpt("Signed off.", "signed", 40);
        assert_eq!(out, "Signed off.", "nothing was cut, so nothing should say it was");
    }

    #[test]
    fn a_match_at_the_end_of_a_long_message_is_still_shown() {
        let body = format!("{} the pineapple", "filler ".repeat(60));
        let out = excerpt(&body, "pineapple", 30);
        assert!(out.contains("pineapple"), "expected the tail of the message, got {out}");
        assert!(out.starts_with('…'), "and an ellipsis saying where it came from: {out}");
    }

    #[test]
    fn an_excerpt_never_splits_a_character() {
        // Cut by byte offset this panics rather than returning a short line,
        // and it panics inside a search rather than near the message.
        let body = "😀".repeat(200);
        let out = excerpt(&body, "😀", 20);
        assert!(out.chars().count() <= 22, "{out}");
        assert!(out.contains('😀'));
    }

    #[test]
    fn a_bare_url_is_found() {
        assert_eq!(
            links_in("see https://example.com/report for the numbers"),
            ["https://example.com/report"]
        );
    }

    #[test]
    fn a_markdown_link_does_not_keep_its_closing_bracket() {
        assert_eq!(links_in("[the report](https://example.com/q3)"), ["https://example.com/q3"]);
    }

    #[test]
    fn a_url_that_contains_brackets_keeps_them() {
        // The Wikipedia case. Trimming every trailing bracket breaks this one,
        // and keeping every one breaks the Markdown case above.
        assert_eq!(
            links_in("https://en.wikipedia.org/wiki/Avocado_(disambiguation)"),
            ["https://en.wikipedia.org/wiki/Avocado_(disambiguation)"]
        );
    }

    #[test]
    fn sentence_punctuation_after_a_url_is_not_part_of_it() {
        assert_eq!(links_in("filed at https://example.com/x."), ["https://example.com/x"]);
        assert_eq!(links_in("<https://example.com/y>"), ["https://example.com/y"]);
    }

    #[test]
    fn several_urls_in_one_message_are_all_found() {
        let text = "http://a.example/one and https://b.example/two";
        assert_eq!(links_in(text), ["http://a.example/one", "https://b.example/two"]);
    }

    #[test]
    fn the_word_http_is_not_a_link() {
        assert!(links_in("we spoke about http and https, nothing more").is_empty());
        assert!(links_in("https://").is_empty());
    }
}
