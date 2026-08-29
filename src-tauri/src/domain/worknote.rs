//! A working note: one line about what an agent is in the middle of.
//!
//! The counterpart to memory, and defined by what memory is not. Memory is a
//! page of durable belief the agent rewrites whole. A working note is a line
//! about right now, appended and never revised, that falls off on its own.
//!
//! The split exists because one store cannot have both lifetimes. Given only
//! memory, an agent puts progress in it, and correctly so: the alternative to
//! writing down what it is waiting on is forgetting it, and forgetting it is
//! the worse failure. What it costs is that progress written into a page that
//! is rewritten every turn never leaves. `Rewriting` a page is the cheapest
//! moment to copy a stale section forward and the most expensive moment to
//! decide it is stale, so the ratchet only turns one way.
//!
//! Two findings shape the write rules, and they point in opposite directions,
//! which is why the two stores are not symmetric:
//!
//! - Consolidating a memory on every interaction degrades it, and past a point
//!   degrades it below having no memory at all (arXiv 2605.12978). The fix is
//!   to gate consolidation rather than fire it after every turn. Memory is
//!   small and written rarely, so it can afford to be reconciled; a working
//!   note is never consolidated at all.
//! - Localized maintenance costs less and holds up better than global
//!   reorganization (arXiv 2606.24775). A full rewrite is global reorganization
//!   by definition. An append is as localized as a write gets.
//!
//! Which leaves forgetting. It has to happen, and the agent must not be the one
//! doing it: a stale note does not sit inert, it steers the next turn toward
//! work that is already done (arXiv 2505.16067). So the store forgets by age,
//! by itself, and nothing in the tool surface offers an agent a way to revise
//! what it wrote. Deciding what to drop is the operation these models are
//! measurably worst at, and this is the one store that never asks.

use serde::{Deserialize, Serialize};

/// How many of an agent's notes survive. The oldest fall off past this.
///
/// Sized against what it is for rather than against a context budget: what an
/// agent is in the middle of is a handful of things, and an agent that needs
/// twenty lines to say what it is doing is describing a conversation instead of
/// its state. Small enough that the whole list is worth reading every turn,
/// which is what makes it a state and not an archive.
pub const KEPT: usize = 16;

/// How long one note may be. A line, not a paragraph.
///
/// The cap is the specification. A note is "waiting on Robert's decision on the
/// six items" or "handed the scope doc to the PM", and anything that does not
/// fit is either a document, which belongs in a document, or a durable fact,
/// which belongs in memory. Given room for a paragraph an agent writes one, and
/// sixteen paragraphs is the page this store exists to stop being written.
pub const MAX_NOTE: usize = 240;

/// What became of a note the agent wrote.
///
/// A line the agent already holds is not stored a second time, and the agent is
/// told so rather than told "noted". The difference is the whole point: an
/// agent that gets an acknowledgment for restating a note learns that restating
/// is how you say something is still true, and a bounded list then fills with
/// one fact written six ways. It is not a revision either, which is the rule
/// this store does keep: nothing is edited and nothing is dropped, the second
/// write simply never happens.
///
/// The stamp carried here is the note's original one, and it is deliberately
/// not moved forward. The age is what makes the list worth reading, so an agent
/// that renotes what it noted three days ago must be told it has been three
/// days, not handed a fresh clock that hides it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appended {
    /// The note is new and is now in the list.
    Stored,
    /// The agent already holds this exact line, written at this stamp.
    AlreadyHeld { at: i64 },
}

/// One note, as it is stored and as it is read back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkingNote {
    /// Milliseconds since the epoch, and the reason a note can be judged stale.
    ///
    /// Shown to the agent as an age rather than kept to itself. A note with no
    /// date reads as current forever, which is the failure this store is meant
    /// to avoid rather than reproduce in a second place.
    pub at: i64,
    pub body: String,
}

/// What is actually stored for a note, which is the input trimmed and cut.
///
/// Returned rather than applied silently for the reason `Workspace::write`
/// hands back its own truncation: an agent that believes it recorded something
/// it did not will not write it again.
pub fn store_as(body: &str) -> (String, bool) {
    // Cut on a word so the fragment that survives is still readable. A note is
    // one line, so there is no line boundary to fall back on the way memory has.
    super::cut_to(body, MAX_NOTE)
}

/// How long ago, in the coarsest unit that is still true.
///
/// The unit is the point. An agent reading "3 days ago" beside "waiting on the
/// operator's decision" has what it needs to stop waiting; the same note
/// stamped with an ISO timestamp needs the model to do date arithmetic against
/// a clock it has to be told, and it gets it wrong. Coarse and correct beats
/// precise and re-derived.
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
        // Silently keeping half of it would let an agent believe it had
        // recorded the half that mattered.
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
        // No whitespace to fall back on, and a note that cannot be cut is a
        // note that defeats the cap.
        let (body, cut) = store_as(&"x".repeat(MAX_NOTE * 2));
        assert!(cut);
        assert_eq!(body.chars().count(), MAX_NOTE);
    }

    #[test]
    fn the_cap_counts_characters_rather_than_bytes() {
        // Same rule `Workspace::write` follows. A note of emoji is a quarter of
        // the bytes it looks like and must not be cut four times as early.
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
        // Two clocks and a note written a moment ago on the far side of a
        // system time change. "-1d ago" is worse than slightly wrong.
        let now = 1_000_000_000_000;
        assert_eq!(how_long_ago(now + 60_000, now), "just now");
    }
}
