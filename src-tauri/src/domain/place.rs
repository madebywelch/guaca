use serde::{Deserialize, Serialize};

use super::agent::{AgentCard, Lifecycle};
use super::approval::{ApprovalState, Decision, ProtectedAction};
use super::envelope::Trust;
use super::ids::AgentId;
use super::outcome::Status;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AdapterId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PlaceKey(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ThreadKey(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HeardKey(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OutsideUser(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Posted(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RelayTag(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Spot {
    pub place: PlaceKey,
    pub thread: Option<ThreadKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Heard {
    pub key: HeardKey,
    pub spot: Spot,
    pub direct: bool,
    pub speaker: OutsideUser,
    pub at_ms: i64,
    pub said: Said,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Said {
    Text { text: String },
    Choice { ask: String, choice: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CardKind {
    Reply,
    Question,
    Permission,
    Notice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChoiceStyle {
    Primary,
    Plain,
    Danger,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    pub id: String,
    pub label: String,
    pub style: ChoiceStyle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub kind: CardKind,
    pub from: Option<String>,
    pub text: String,
    pub fields: Vec<(String, String)>,
    pub ask: Option<String>,
    pub choices: Vec<Choice>,
    pub settled: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Buttons {
    pub max: usize,
    pub label_chars: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PostSafety {
    Idempotent,
    Findable,
    Neither,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Abilities {
    pub buttons: Option<Buttons>,
    pub edits: bool,
    pub reactions: bool,
    pub max_text: usize,
    pub posts: PostSafety,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mark {
    Working,
    Done,
    NeedsYou,
    Failed,
}

impl Mark {
    pub fn as_str(self) -> &'static str {
        match self {
            Mark::Working => "working",
            Mark::Done => "done",
            Mark::NeedsYou => "needsYou",
            Mark::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Outgoing {
    Post { card: Card },
    Edit { posted: Posted, card: Card },
    Mark { heard: HeardKey, mark: Mark },
    Skip,
}

pub const STRANGER: &str = "This Guaca answers its owner.";
pub const UNBOUND: &str =
    "This conversation is not connected to a group yet. Connect it in Guaca's Settings.";
pub const CODE_IN_CHANNEL: &str =
    "Send the code to the Guaca bot in a direct message. A code sent here links nothing.";
pub const NOT_FROM_HERE: &str =
    "That request was not asked here, so it can only be answered in Guaca.";
pub const AGENT_UNAVAILABLE: &str = "This agent is unavailable. Pick another in Guaca's Settings.";
pub const TRY_AGAIN: &str = "Try again in Guaca.";
pub const ALLOW: &str = "allow";
pub const ALWAYS_ALLOW: &str = "alwaysAllow";
pub const DENY: &str = "deny";

pub const LATE_MS: i64 = 5 * 60 * 1000;
pub const STALE_MS: i64 = 24 * 60 * 60 * 1000;
pub const LINK_CODE_MS: i64 = 10 * 60 * 1000;
const QUOTE_CHARS: usize = 60;

pub fn linked(agent: &str) -> String {
    format!("Linked. Messages here go to {agent}.")
}

pub fn waiting_for_owner(agent: &str) -> String {
    format!("{agent} is waiting for its owner in a direct message.")
}

pub fn quote(text: &str) -> String {
    let line = text.trim().lines().next().unwrap_or_default();
    let (kept, cut) = super::cut_to(line, QUOTE_CHARS);
    let more = cut || text.trim().lines().nth(1).is_some();
    format!("\"{kept}{}\"", if more { "..." } else { "" })
}

pub fn offline_notice(text: &str) -> Card {
    notice(format!("Guaca was offline when you sent {}. Answering it now.", quote(text)))
}

pub fn stale_notice(text: &str) -> Card {
    notice(format!(
        "Guaca was offline when you sent {}, more than a day ago. Ask again if it still matters.",
        quote(text)
    ))
}

pub fn failed_notice(headline: &str) -> Card {
    let headline = headline.trim().trim_end_matches('.');
    if headline.is_empty() {
        return notice(format!("This did not finish. {TRY_AGAIN}"));
    }
    notice(format!("{headline}. {TRY_AGAIN}"))
}

pub fn stale(at_ms: i64, now: i64) -> bool {
    now - at_ms > STALE_MS
}

pub fn late(at_ms: i64, first_post_at: i64) -> bool {
    first_post_at - at_ms > LATE_MS
}

pub fn trust(person: Option<&OutsideUser>, speaker: &OutsideUser) -> Option<Trust> {
    (person == Some(speaker)).then_some(Trust::Operator)
}

pub fn mark_for(status: Status) -> Mark {
    match status {
        Status::Done | Status::NothingNew | Status::Stopped => Mark::Done,
        Status::NeedsYou => Mark::NeedsYou,
        Status::Failed | Status::Interrupted => Mark::Failed,
    }
}

pub fn route(text: &str, agents: &[AgentCard], default: AgentId) -> (AgentId, String) {
    let whole = || (default, text.trim().to_string());
    let Some(rest) = text.strip_prefix('@') else { return whole() };
    let named = agents
        .iter()
        .filter(|agent| agent.lifecycle != Lifecycle::Terminated)
        .filter(|agent| {
            let name = agent.name.to_lowercase();
            let head: String = rest.chars().take(name.chars().count()).collect();
            head.to_lowercase() == name
                && rest[head.len()..].chars().next().is_some_and(char::is_whitespace)
        })
        .max_by_key(|agent| agent.name.chars().count());
    let Some(agent) = named else { return whole() };
    let head_len: usize = rest.chars().take(agent.name.chars().count()).map(char::len_utf8).sum();
    let body = rest[head_len..].trim();
    if body.is_empty() {
        return whole();
    }
    (agent.id, body.to_string())
}

pub fn reply(from: &str, text: &str) -> Card {
    Card {
        kind: CardKind::Reply,
        from: Some(from.to_string()),
        text: text.to_string(),
        fields: Vec::new(),
        ask: None,
        choices: Vec::new(),
        settled: None,
    }
}

pub fn question(from: &str, ask: &str, text: &str, options: &[String]) -> Card {
    Card {
        kind: CardKind::Question,
        from: Some(from.to_string()),
        text: text.to_string(),
        fields: Vec::new(),
        ask: Some(ask.to_string()),
        choices: options
            .iter()
            .enumerate()
            .map(|(at, label)| Choice {
                id: at.to_string(),
                label: label.clone(),
                style: ChoiceStyle::Plain,
            })
            .collect(),
        settled: None,
    }
}

pub fn permission(
    from: &str,
    ask: &str,
    text: &str,
    fields: Vec<(String, String)>,
    action: ProtectedAction,
) -> Card {
    let choice =
        |id: &str, label: &str, style| Choice { id: id.into(), label: label.into(), style };
    let mut choices = vec![choice(ALLOW, "Allow", ChoiceStyle::Primary)];
    if action.stands() {
        choices.push(choice(ALWAYS_ALLOW, "Always allow", ChoiceStyle::Plain));
    }
    choices.push(choice(DENY, "Deny", ChoiceStyle::Danger));
    Card {
        kind: CardKind::Permission,
        from: Some(from.to_string()),
        text: text.to_string(),
        fields,
        ask: Some(ask.to_string()),
        choices,
        settled: None,
    }
}

pub fn notice(text: String) -> Card {
    Card {
        kind: CardKind::Notice,
        from: None,
        text,
        fields: Vec::new(),
        ask: None,
        choices: Vec::new(),
        settled: None,
    }
}

pub fn settled(state: ApprovalState, answer: Option<&str>, agent: &str) -> Option<String> {
    match state {
        ApprovalState::Pending => None,
        ApprovalState::Allow => Some("Allowed".into()),
        ApprovalState::AlwaysAllow => Some("Always allowed".into()),
        ApprovalState::Deny => Some("Denied".into()),
        ApprovalState::Answered => Some(format!("Answered: {}", answer.unwrap_or_default().trim())),
        ApprovalState::Expired => Some(format!("No answer in ten minutes, {agent} went on")),
    }
}

pub fn settle(card: Card, settled: String) -> Card {
    Card { choices: Vec::new(), settled: Some(settled), ..card }
}

pub fn decision_for(choice: &str) -> Option<Decision> {
    match choice {
        ALLOW => Some(Decision::Allow),
        ALWAYS_ALLOW => Some(Decision::AlwaysAllow),
        DENY => Some(Decision::Deny),
        _ => None,
    }
}

pub fn option_for<'a>(choice: &str, options: &'a [String]) -> Option<&'a str> {
    options.get(choice.parse::<usize>().ok()?).map(String::as_str)
}

pub fn read_choice<'a>(reply: &str, choices: &'a [Choice]) -> Option<&'a Choice> {
    let said = reply.trim().trim_end_matches('.').trim();
    if let Ok(number) = said.parse::<usize>() {
        return number.checked_sub(1).and_then(|at| choices.get(at));
    }
    choices.iter().find(|choice| choice.label.to_lowercase() == said.to_lowercase())
}

fn needs_numbers(card: &Card, abilities: &Abilities) -> bool {
    if card.choices.is_empty() {
        return false;
    }
    match abilities.buttons {
        None => true,
        Some(buttons) => {
            card.choices.len() > buttons.max
                || card
                    .choices
                    .iter()
                    .any(|choice| choice.label.chars().count() > buttons.label_chars)
        }
    }
}

fn numbered(card: Card) -> Card {
    let lines: Vec<String> = card
        .choices
        .iter()
        .enumerate()
        .map(|(at, choice)| format!("{}. {}", at + 1, choice.label))
        .collect();
    let text = format!(
        "{}\n\n{}\n\nReply with a number or the words of a choice.",
        card.text.trim_end(),
        lines.join("\n")
    );
    Card { text, choices: Vec::new(), ..card }
}

pub fn fit(card: Card, abilities: &Abilities) -> Vec<Card> {
    let card = if needs_numbers(&card, abilities) { numbered(card) } else { card };
    let pieces = split(&card.text, abilities.max_text);
    let last = pieces.len() - 1;
    pieces
        .into_iter()
        .enumerate()
        .map(|(at, text)| {
            if at == last {
                Card { text, ..card.clone() }
            } else {
                Card {
                    kind: card.kind,
                    from: card.from.clone(),
                    text,
                    fields: Vec::new(),
                    ask: None,
                    choices: Vec::new(),
                    settled: None,
                }
            }
        })
        .collect()
}

pub fn split(text: &str, max: usize) -> Vec<String> {
    assert!(max > 0, "an adapter declares room for at least one character");
    if text.chars().count() <= max {
        return vec![text.to_string()];
    }
    let mut pieces = Vec::new();
    let mut rest = text.trim();
    while rest.chars().count() > max {
        let (end, next) = break_at(rest, max);
        pieces.push(rest[..end].trim_end().to_string());
        rest = &rest[next..];
    }
    pieces.push(rest.to_string());
    pieces
}

fn break_at(text: &str, max: usize) -> (usize, usize) {
    let end = |chars: usize| text.char_indices().nth(chars).map(|(at, _)| at).unwrap_or(text.len());
    let limit = end(max);
    for gap in ["\n\n", "\n", " "] {
        let window = &text[..end(max + gap.len())];
        if let Some(at) = window.rfind(gap).filter(|at| *at > 0 && *at <= limit) {
            let next = at + gap.len();
            let skip = text[next..].len() - text[next..].trim_start_matches(['\n', ' ']).len();
            return (at, next + if gap == " " { 0 } else { skip });
        }
    }
    (limit, limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::Lifecycle;

    fn agent(name: &str) -> AgentCard {
        let mut card: AgentCard = serde_json::from_value(serde_json::json!({
            "id": AgentId::new(),
            "groupId": crate::domain::ids::GroupId::new(),
            "name": name,
            "avatar": "avocado",
            "color": "#7fb069",
            "model": "test/model",
            "systemPrompt": "",
            "skills": [],
            "sandboxId": null,
            "browserId": null,
            "hasComputer": false,
            "hasBrowser": false,
            "browserConsent": "open",
            "runsErrands": false,
            "lifecycle": "active",
            "pinned": false,
            "railOrder": 0,
            "version": 1,
            "createdAt": 0,
            "updatedAt": 0,
            "discardedAt": null
        }))
        .expect("a minimal agent card");
        card.lifecycle = Lifecycle::Active;
        card
    }

    fn abilities(buttons: Option<Buttons>, max_text: usize) -> Abilities {
        Abilities { buttons, edits: true, reactions: true, max_text, posts: PostSafety::Findable }
    }

    #[test]
    fn a_text_without_a_leading_name_goes_to_the_default_agent_whole() {
        let crew = vec![agent("Pip"), agent("Ada")];
        let default = crew[0].id;
        assert_eq!(route("hello @Ada", &crew, default), (default, "hello @Ada".into()));
        assert_eq!(route("@Nobody hi", &crew, default), (default, "@Nobody hi".into()));
        assert_eq!(route("@Ada", &crew, default), (default, "@Ada".into()));
        assert_eq!(route("@Adam hi", &crew, default), (default, "@Adam hi".into()));
    }

    #[test]
    fn a_leading_name_routes_to_that_agent_case_insensitively_and_the_longest_name_wins() {
        let crew = vec![agent("Pip"), agent("Ada"), agent("Ada Lovelace")];
        let default = crew[0].id;
        assert_eq!(route("@ada  check this ", &crew, default), (crew[1].id, "check this".into()));
        assert_eq!(route("@Ada Lovelace hi", &crew, default), (crew[2].id, "hi".into()));
    }

    #[test]
    fn a_deleted_agent_is_never_routed_to() {
        let mut crew = vec![agent("Pip"), agent("Ada")];
        crew[1].lifecycle = Lifecycle::Terminated;
        let default = crew[0].id;
        assert_eq!(route("@Ada hi", &crew, default), (default, "@Ada hi".into()));
    }

    #[test]
    fn only_the_linked_person_is_trusted() {
        let person = OutsideUser("U1".into());
        assert_eq!(trust(Some(&person), &OutsideUser("U1".into())), Some(Trust::Operator));
        assert_eq!(trust(Some(&person), &OutsideUser("U2".into())), None);
        assert_eq!(trust(None, &OutsideUser("U1".into())), None);
    }

    #[test]
    fn choices_without_buttons_become_numbered_text() {
        let card = question("Pip", "a1", "Ship it?", &["Ship it".into(), "Wait".into()]);
        let fitted = fit(card, &abilities(None, 1000));
        assert_eq!(fitted.len(), 1);
        assert!(fitted[0].choices.is_empty());
        assert_eq!(fitted[0].ask.as_deref(), Some("a1"));
        assert!(fitted[0].text.contains("1. Ship it\n2. Wait"), "{}", fitted[0].text);
    }

    #[test]
    fn too_few_buttons_or_long_labels_become_numbered_text_and_enough_stay_buttons() {
        let card = question("Pip", "a1", "Pick", &["One".into(), "Two".into(), "Three".into()]);
        let few = fit(card.clone(), &abilities(Some(Buttons { max: 2, label_chars: 20 }), 1000));
        assert!(few[0].choices.is_empty());
        let short = fit(card.clone(), &abilities(Some(Buttons { max: 5, label_chars: 3 }), 1000));
        assert!(short[0].choices.is_empty());
        let room = fit(card, &abilities(Some(Buttons { max: 5, label_chars: 20 }), 1000));
        assert_eq!(room[0].choices.len(), 3);
        assert_eq!(room[0].text, "Pick");
    }

    #[test]
    fn a_reply_reads_as_a_choice_by_number_or_label() {
        let card = permission("Pip", "a1", "Hire?", Vec::new(), ProtectedAction::CreateAgent);
        let ids = |reply: &str| read_choice(reply, &card.choices).map(|c| c.id.clone());
        assert_eq!(ids("1"), Some(ALLOW.into()));
        assert_eq!(ids(" 3. "), Some(DENY.into()));
        assert_eq!(ids("always ALLOW"), Some(ALWAYS_ALLOW.into()));
        assert_eq!(ids("0"), None);
        assert_eq!(ids("4"), None);
        assert_eq!(ids("sure"), None);
    }

    #[test]
    fn always_allow_is_offered_only_for_an_action_that_stands() {
        let standing = permission("Pip", "a", "x", Vec::new(), ProtectedAction::CreateAgent);
        let once = permission("Pip", "a", "x", Vec::new(), ProtectedAction::ActOnBehalf);
        let ids = |card: &Card| card.choices.iter().map(|c| c.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&standing), vec![ALLOW, ALWAYS_ALLOW, DENY]);
        assert_eq!(ids(&once), vec![ALLOW, DENY]);
        assert_eq!(once.choices[1].style, ChoiceStyle::Danger);
        assert!(once.choices.iter().all(|c| decision_for(&c.id).is_some()));
    }

    #[test]
    fn a_question_choice_id_is_the_index_of_its_option() {
        let options = vec!["Ship it".to_string(), "Wait".to_string()];
        let card = question("Pip", "a", "x", &options);
        assert_eq!(option_for(&card.choices[1].id, &options), Some("Wait"));
        assert_eq!(option_for("2", &options), None);
        assert_eq!(option_for("allow", &options), None);
    }

    #[test]
    fn long_text_splits_at_paragraphs_before_lines_before_words_in_order() {
        let text = "first paragraph here\n\nsecond line one\nsecond line two";
        assert_eq!(
            split(text, 34),
            vec!["first paragraph here", "second line one\nsecond line two"]
        );
        assert_eq!(split("aaa bbb\nccc ddd", 10), vec!["aaa bbb", "ccc ddd"]);
        assert_eq!(split("aaa bbb ccc", 8), vec!["aaa bbb", "ccc"]);
        assert_eq!(split("epsilon zeta eta", 12), vec!["epsilon zeta", "eta"]);
        assert_eq!(split("one\n\ntwo three", 3), vec!["one", "two", "thr", "ee"]);
        assert_eq!(split("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(split("short", 10), vec!["short"]);
        assert_eq!(split("  code\n", 10), vec!["  code\n"]);
    }

    #[test]
    fn every_piece_fits_and_nothing_is_lost() {
        let text = "Lorem ipsum dolor sit amet.\n\nConsectetur adipiscing elit, sed do eiusmod.\n\
                    Tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam.";
        for max in [5, 12, 30, 64] {
            let pieces = split(text, max);
            assert!(pieces.iter().all(|p| p.chars().count() <= max), "{max}: {pieces:?}");
            let words: Vec<&str> = text.split_whitespace().collect();
            let joined = pieces.join(" ");
            let back: Vec<&str> = joined.split_whitespace().collect();
            if max >= 12 {
                assert_eq!(back, words, "{max}");
            }
        }
    }

    #[test]
    fn splitting_a_card_keeps_its_ask_and_choices_on_the_last_piece() {
        let card = permission(
            "Pip",
            "a1",
            "One two three four five six",
            vec![("Name".into(), "Ada".into())],
            ProtectedAction::ActOnBehalf,
        );
        let fitted = fit(card, &abilities(Some(Buttons { max: 3, label_chars: 20 }), 10));
        assert!(fitted.len() > 1);
        let (last, rest) = fitted.split_last().unwrap();
        assert!(rest
            .iter()
            .all(|c| c.ask.is_none() && c.choices.is_empty() && c.fields.is_empty()));
        assert_eq!(last.ask.as_deref(), Some("a1"));
        assert_eq!(last.choices.len(), 2);
        assert_eq!(last.fields.len(), 1);
        assert!(fitted
            .iter()
            .all(|c| c.kind == CardKind::Permission && c.from.as_deref() == Some("Pip")));
    }

    #[test]
    fn a_message_older_than_a_day_is_stale_and_an_answer_after_five_minutes_is_late() {
        assert!(!stale(0, STALE_MS));
        assert!(stale(0, STALE_MS + 1));
        assert!(!late(0, LATE_MS));
        assert!(late(0, LATE_MS + 1));
    }

    #[test]
    fn notices_come_from_guaca_and_quote_the_start_of_the_message() {
        let card = offline_notice("Can you book the room for Friday?\nAnd lunch.");
        assert_eq!(card.from, None);
        assert_eq!(card.kind, CardKind::Notice);
        assert!(card.text.contains("\"Can you book the room for Friday?...\""), "{}", card.text);
        assert!(stale_notice("hi").text.contains("Ask again if it still matters."));
        assert_eq!(
            failed_notice("The model refused.").text,
            "The model refused. Try again in Guaca."
        );
    }

    #[test]
    fn what_a_settled_ask_says() {
        assert_eq!(settled(ApprovalState::Pending, None, "Pip"), None);
        assert_eq!(
            settled(ApprovalState::Answered, Some("Ship it"), "Pip").unwrap(),
            "Answered: Ship it"
        );
        assert!(settled(ApprovalState::Expired, None, "Pip").unwrap().contains("Pip went on"));
        let card = settle(question("Pip", "a", "x", &["A".into(), "B".into()]), "Denied".into());
        assert!(card.choices.is_empty());
        assert_eq!(card.settled.as_deref(), Some("Denied"));
    }

    #[test]
    fn an_outcome_maps_to_one_mark() {
        assert_eq!(mark_for(Status::Done), Mark::Done);
        assert_eq!(mark_for(Status::NeedsYou), Mark::NeedsYou);
        assert_eq!(mark_for(Status::Interrupted), Mark::Failed);
        assert_eq!(mark_for(Status::Failed), Mark::Failed);
    }
}
