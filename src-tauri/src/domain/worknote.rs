use serde::{Deserialize, Serialize};

pub const KEPT: usize = 16;

pub const MAX_NOTE: usize = 240;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appended {
    Stored,
    AlreadyHeld { at: i64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkingNote {
    pub at: i64,
    pub body: String,
}

pub fn store_as(body: &str) -> (String, bool) {
    super::cut_to(body, MAX_NOTE)
}

pub fn how_long_ago(at: i64, now: i64) -> String {
    let ms = (now - at).max(0);
    let minutes = ms / 60_000;
    let hours = minutes / 60;
    let days = hours / 24;
    if days >= 1 {
        format!("{days}d ago")
    } else if hours >= 1 {
        format!("{hours}h ago")
    } else if minutes >= 1 {
        format!("{minutes}m ago")
    } else {
        "just now".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_within_the_cap_is_stored_whole() {
        let (body, cut) = store_as("  waiting on the operator's decision  ");
        assert_eq!(body, "waiting on the operator's decision");
        assert!(!cut, "trimming is not a cut");
    }

    #[test]
    fn an_oversized_note_is_cut_and_says_so() {
        let long = "word ".repeat(200);
        let (body, cut) = store_as(&long);
        assert!(cut);
        assert!(body.chars().count() <= MAX_NOTE);
        assert!(!body.ends_with(' '), "cut left trailing space: {body:?}");
    }

    #[test]
    fn a_cut_lands_on_a_word() {
        let long = format!("{} finalword", "padding ".repeat(40));
        let (body, _) = store_as(&long);
        assert!(body.ends_with("padding"), "cut mid-word: {body:?}");
    }

    #[test]
    fn a_single_long_word_is_still_cut_to_the_cap() {
        let (body, cut) = store_as(&"x".repeat(MAX_NOTE * 2));
        assert!(cut);
        assert_eq!(body.chars().count(), MAX_NOTE);
    }

    #[test]
    fn the_cap_counts_characters_rather_than_bytes() {
        let (body, cut) = store_as(&"🥑".repeat(MAX_NOTE));
        assert!(!cut);
        assert_eq!(body.chars().count(), MAX_NOTE);
    }

    #[test]
    fn an_age_is_the_coarsest_unit_that_is_true() {
        let now = 1_000_000_000_000;
        assert_eq!(how_long_ago(now, now), "just now");
        assert_eq!(how_long_ago(now - 90_000, now), "1m ago");
        assert_eq!(how_long_ago(now - 3 * 3_600_000, now), "3h ago");
        assert_eq!(how_long_ago(now - 50 * 3_600_000, now), "2d ago");
    }

    #[test]
    fn a_note_from_the_future_reads_as_current_rather_than_negative() {
        let now = 1_000_000_000_000;
        assert_eq!(how_long_ago(now + 60_000, now), "just now");
    }
}
