use super::envelope::{Envelope, Part};
use super::ids::{AgentId, MessageId, RunId};

pub const SHOWN: usize = 5;

pub const SHOWN_FOR_ROUTINE: usize = 10;

pub const CANDIDATES: u32 = 40;

pub const QUERY_TERMS: usize = 12;

pub const MIN_TERM_CHARS: usize = 2;

pub const WHOLE_CHARS: usize = 400;

pub const EXCERPT_TOKENS: u32 = 48;

pub const ELLIPSIS: &str = "…";

const STOP_WORDS: &[&str] = &[
    "a", "about", "after", "again", "all", "also", "am", "an", "and", "any", "anything", "are",
    "as", "at", "be", "because", "been", "before", "being", "but", "by", "can", "could", "did",
    "do", "does", "doing", "done", "for", "from", "get", "got", "had", "has", "have", "he", "her",
    "here", "him", "his", "how", "i", "if", "in", "into", "is", "it", "its", "just", "know", "let",
    "like", "me", "more", "my", "no", "not", "now", "of", "ok", "okay", "on", "one", "or", "our",
    "out", "please", "re", "she", "should", "so", "some", "than", "thanks", "that", "the", "their",
    "them", "then", "there", "these", "they", "this", "those", "to", "too", "up", "us", "very",
    "was", "we", "were", "what", "when", "where", "which", "who", "why", "will", "with", "would",
    "yes", "you", "your",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub run_id: RunId,
    pub agent_id: AgentId,
    pub message_id: MessageId,
    pub said: String,
    pub headline: String,
    pub said_at: i64,
}

pub fn asked(envelope: &Envelope) -> Vec<String> {
    envelope
        .parts
        .iter()
        .filter_map(|part| match part {
            Part::Text { text } => Some(text.clone()),
            Part::Routine { name, what, .. } => Some(format!("{name}\n{what}")),
            _ => None,
        })
        .collect()
}

pub fn terms(texts: &[&str]) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for text in texts {
        for word in text.split(|c: char| !c.is_alphanumeric()) {
            let word = word.to_lowercase();
            if word.chars().count() < MIN_TERM_CHARS
                || STOP_WORDS.contains(&word.as_str())
                || seen.contains(&word)
            {
                continue;
            }
            seen.push(word);
        }
    }
    let mut ranked: Vec<(usize, String)> = seen.into_iter().enumerate().collect();
    ranked.sort_by(|(a_at, a), (b_at, b)| {
        b.chars().count().cmp(&a.chars().count()).then(a_at.cmp(b_at))
    });
    ranked.truncate(QUERY_TERMS);
    ranked.sort_by_key(|(at, _)| *at);
    ranked.into_iter().map(|(_, word)| word).collect()
}

pub fn query(texts: &[&str]) -> Option<String> {
    let terms = terms(texts);
    (!terms.is_empty())
        .then(|| terms.iter().map(|term| format!("\"{term}\"")).collect::<Vec<_>>().join(" OR "))
}

pub fn shown_for(routine: bool) -> usize {
    if routine {
        SHOWN_FOR_ROUTINE
    } else {
        SHOWN
    }
}

pub fn excerpt(said: &str, snippet: &str) -> String {
    let said = said.split_whitespace().collect::<Vec<_>>().join(" ");
    if said.chars().count() <= WHOLE_CHARS {
        return said;
    }
    snippet.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn pick(ranked: Vec<Card>, shown_already: impl Fn(&Card) -> bool, limit: usize) -> Vec<Card> {
    let mut picked: Vec<Card> = ranked.into_iter().filter(|card| !shown_already(card)).collect();
    picked.truncate(limit);
    picked.sort_by(|a, b| b.said_at.cmp(&a.said_at).then(b.run_id.cmp(&a.run_id)));
    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(said: &str, at: i64) -> Card {
        Card {
            run_id: RunId::new(),
            agent_id: AgentId::new(),
            message_id: MessageId::new(),
            said: said.into(),
            headline: String::new(),
            said_at: at,
        }
    }

    #[test]
    fn a_question_made_only_of_small_words_searches_for_nothing() {
        assert_eq!(query(&["What is it? Can you do it now, please?"]), None);
        assert_eq!(query(&[""]), None);
    }

    #[test]
    fn a_query_keeps_the_words_that_carry_the_question_and_no_operator_syntax() {
        assert_eq!(
            query(&["What should I cook on Saturday?"]).as_deref(),
            Some("\"cook\" OR \"saturday\"")
        );
        let hostile = query(&["NEAR(\"a\" b) OR title:x* AND -y ^z"]).unwrap();
        assert!(!hostile.contains('*') && !hostile.contains(':') && !hostile.contains('^'));
        assert!(hostile.split(" OR ").all(|t| t.starts_with('"') && t.ends_with('"')));
    }

    #[test]
    fn a_long_message_is_cut_to_its_longest_words_in_the_order_they_were_said() {
        let long = "one two three alpha bravo charlie delta echo foxtrot golf hotel india \
                    juliett kilo lima mike november oscar papa quebec romeo sierra tango";
        let picked = terms(&[long]);
        assert_eq!(picked.len(), QUERY_TERMS);
        assert!(picked.contains(&"november".to_string()));
        assert!(!picked.contains(&"two".to_string()));
        let positions: Vec<usize> = picked.iter().map(|t| long.find(t.as_str()).unwrap()).collect();
        assert!(positions.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn a_repeated_word_is_one_term_whatever_its_case() {
        assert_eq!(terms(&["Lisbon lisbon LISBON", "lisbon"]), vec!["lisbon".to_string()]);
    }

    #[test]
    fn a_short_message_is_shown_whole_and_a_long_one_by_the_passage_that_matched() {
        assert_eq!(
            excerpt("  I am  allergic\nto shellfish. ", "ignored"),
            "I am allergic to shellfish."
        );
        let long = "x ".repeat(WHOLE_CHARS);
        assert_eq!(excerpt(&long, "…the gate code is 4471…"), "…the gate code is 4471…");
    }

    #[test]
    fn what_is_already_in_front_of_the_agent_is_skipped_and_the_rest_is_newest_first() {
        let old = card("old", 1);
        let seen = card("seen", 2);
        let new = card("new", 3);
        let worst = card("worst", 4);
        let seen_id = seen.message_id;
        let picked =
            pick(vec![old.clone(), seen, new.clone(), worst], |card| card.message_id == seen_id, 2);
        assert_eq!(picked, vec![new, old], "best two by rank, then newest first");
    }
}
