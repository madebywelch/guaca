const STARTING: &[&str] = &[
    "checking",
    "rechecking",
    "re-checking",
    "double-checking",
    "verifying",
    "confirming",
    "testing",
    "retesting",
    "re-testing",
    "running",
    "rerunning",
    "re-running",
    "pulling",
    "fetching",
    "querying",
    "sweeping",
    "kicking off",
    "chasing",
];

const LEADS: &[&str] = &[
    "i'll ",
    "i will ",
    "i am going to ",
    "i'm going to ",
    "i am about to ",
    "i'm about to ",
    "let me ",
    "going to ",
    "about to ",
    "next i ",
    "then i ",
];

const ACTIONS: &[&str] = &[
    "check",
    "verify",
    "confirm",
    "test",
    "run",
    "look",
    "pull",
    "fetch",
    "search",
    "read",
    "open",
    "query",
    "dig",
    "sweep",
    "scan",
    "review",
    "inspect",
    "trace",
    "count",
    "load",
    "grab",
    "chase",
    "find out",
    "figure out",
    "work out",
    "get back",
    "send",
    "write",
    "draft",
    "file",
    "post",
    "email",
    "message",
    "ask",
    "start",
    "kick off",
    "do that",
    "do it",
];

const STALLS: &[&str] = &[
    "one moment",
    "in a moment",
    "bear with",
    "give me a second",
    "give me a minute",
    "give me a moment",
    "coming right up",
];

const STALL_OPENERS: &[&str] = &["on it", "will do", "hold on", "stand by"];

const DEFERRED: &[&str] = &[
    "tomorrow",
    "later",
    "next week",
    "next time",
    "in the morning",
    "overnight",
    "once you",
    "when you",
    "if you",
    "after you",
    "unless you",
    "as soon as you",
    "when it lands",
    "when it comes back",
    "when that comes back",
    "when the job",
    "when it finishes",
    "when this finishes",
];

const OFFERS: &[&str] = &[
    "let me know",
    "want me to",
    "do you want",
    "would you like",
    "should i ",
    "shall i ",
    "happy to",
    "if you'd like",
    "if you want",
];

pub fn promises_work(text: &str) -> Option<&str> {
    let last = last_sentence(text)?;
    let lower = last.to_lowercase();

    if lower.ends_with('?') {
        return None;
    }
    if OFFERS.iter().any(|p| lower.contains(p)) || DEFERRED.iter().any(|p| lower.contains(p)) {
        return None;
    }
    if STALLS.iter().any(|p| lower.contains(p)) {
        return Some(last);
    }
    if STALL_OPENERS.iter().chain(STARTING).any(|opener| lower.starts_with(opener)) {
        return Some(last);
    }
    if LEADS.iter().any(|lead| lower.contains(lead))
        && ACTIONS.iter().any(|action| has_word(&lower, action))
    {
        return Some(last);
    }
    None
}

fn last_sentence(text: &str) -> Option<&str> {
    let trimmed = text.trim_end();
    let mut start = 0;
    let mut back = trimmed.char_indices().rev();
    back.next();
    for (at, c) in back {
        if matches!(c, '.' | '!' | '?' | '\n') {
            start = at + c.len_utf8();
            break;
        }
    }
    let last = trimmed[start..].trim_matches(|c: char| c.is_whitespace() || "-*>#".contains(c));
    (!last.is_empty()).then_some(last)
}

fn has_word(hay: &str, needle: &str) -> bool {
    let mut from = 0;
    while let Some(at) = hay[from..].find(needle) {
        let at = from + at;
        match hay[..at].chars().next_back() {
            Some(before) if before.is_alphanumeric() => from = at + 1,
            _ => return true,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sentence_that_shipped_this_is_caught() {
        assert_eq!(
            promises_work(
                "Both answered — the plugin is back. But the results surfaced two problems I \
                 need to run down. Checking both properly."
            ),
            Some("Checking both properly.")
        );
        assert!(
            promises_work("Retesting both right now — Drive and Gmail, one call each.").is_some()
        );
    }

    #[test]
    fn a_report_of_finished_work_is_not_a_promise() {
        assert_eq!(promises_work("Checked both. Drive is stale and Gmail has nothing."), None);
        assert_eq!(promises_work("I ran the sweep and found six messages."), None);
        assert_eq!(promises_work("Both answered, so the plugin is back."), None);
    }

    #[test]
    fn a_participial_opener_is_a_report_and_not_an_announcement() {
        assert_eq!(promises_work("Looking at the results, Drive is still stale."), None);
        assert_eq!(promises_work("Reading the sweep, nothing from this morning is there."), None);
    }

    #[test]
    fn an_offer_promises_nothing() {
        assert_eq!(
            promises_work("That is everything. Let me know if you want me to dig further."),
            None
        );
        assert_eq!(promises_work("Want me to check the other two?"), None);
        assert_eq!(promises_work("I can run it again if you want."), None);
    }

    #[test]
    fn work_waiting_on_a_person_or_a_day_is_a_state_rather_than_a_promise() {
        assert_eq!(promises_work("I'll check the report tomorrow."), None);
        assert_eq!(promises_work("I'll run the sweep once you confirm the address."), None);
        assert_eq!(promises_work("I'll read the diff when it comes back."), None);
    }

    #[test]
    fn a_stall_needs_no_verb() {
        assert!(promises_work("One moment.").is_some());
        assert!(promises_work("On it.").is_some());
        assert!(promises_work("Will do.").is_some());
    }

    #[test]
    fn a_short_stall_only_counts_where_a_promise_would_stand() {
        assert_eq!(promises_work("Chef is on it."), None);
        assert_eq!(promises_work("There is a hold on that account."), None);
    }

    #[test]
    fn a_lead_needs_a_verb_beside_it() {
        assert_eq!(promises_work("I'll be brief: the plugin is back."), None);
        assert_eq!(promises_work("I'll be honest, that surprised me."), None);
        assert!(promises_work("I'll pull the Drive listing.").is_some());
    }

    #[test]
    fn a_verb_inside_another_word_is_not_a_verb() {
        assert_eq!(promises_work("I'll be the task owner from here."), None);
    }

    #[test]
    fn a_list_marker_does_not_hide_the_closing_line() {
        assert!(promises_work("Two things left:\n- the Drive list\n- Checking both now").is_some());
    }

    #[test]
    fn nothing_to_read_is_not_a_promise() {
        assert_eq!(promises_work(""), None);
        assert_eq!(promises_work("   \n  "), None);
        assert_eq!(promises_work("."), None);
    }
}
