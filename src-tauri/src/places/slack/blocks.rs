use serde_json::{json, Value};

use super::mrkdwn::{escape, mrkdwn};
use crate::domain::cut_to;
use crate::domain::place::{Card, CardKind, ChoiceStyle};

pub const SECTION_CHARS: usize = 3000;
pub const LABEL_CHARS: usize = 75;
pub const BUTTONS: usize = 5;
const FIELDS_PER_SECTION: usize = 10;
const FIELD_CHARS: usize = 2000;
const FALLBACK_CHARS: usize = 300;

pub fn action_id(ask: &str, choice: &str) -> String {
    format!("ask:{ask}:{choice}")
}

pub fn parse_action(action_id: &str) -> Option<(String, String)> {
    let (ask, choice) = action_id.strip_prefix("ask:")?.rsplit_once(':')?;
    (!ask.is_empty() && !choice.is_empty()).then(|| (ask.to_string(), choice.to_string()))
}

pub fn render(card: &Card) -> (Vec<Value>, String) {
    let mut blocks = Vec::new();
    if let Some(from) = &card.from {
        blocks.push(context(&format!("*{}*", escape(from))));
    }
    for part in sections(&mrkdwn(&card.text)) {
        blocks.push(json!({ "type": "section", "text": { "type": "mrkdwn", "text": part } }));
    }
    for fields in card.fields.chunks(FIELDS_PER_SECTION) {
        let fields: Vec<Value> = fields
            .iter()
            .map(|(name, value)| {
                let text = format!("*{}*\n{}", escape(name), escape(value));
                json!({ "type": "mrkdwn", "text": cut_to(&text, FIELD_CHARS).0 })
            })
            .collect();
        blocks.push(json!({ "type": "section", "fields": fields }));
    }
    match (&card.settled, &card.ask) {
        (Some(settled), _) => blocks.push(context(&escape(settled))),
        (None, Some(ask)) if asks(card.kind) && !card.choices.is_empty() => {
            let buttons: Vec<Value> = card
                .choices
                .iter()
                .take(BUTTONS)
                .map(|choice| {
                    let mut button = json!({
                        "type": "button",
                        "text": { "type": "plain_text", "text": label(if choice.label.trim().is_empty() { &choice.id } else { &choice.label }) },
                        "action_id": action_id(ask, &choice.id),
                        "value": choice.id,
                    });
                    match choice.style {
                        ChoiceStyle::Primary => button["style"] = json!("primary"),
                        ChoiceStyle::Danger => button["style"] = json!("danger"),
                        ChoiceStyle::Plain => {}
                    }
                    button
                })
                .collect();
            blocks.push(json!({ "type": "actions", "elements": buttons }));
        }
        (None, _) => {}
    }
    (blocks, fallback(card))
}

fn asks(kind: CardKind) -> bool {
    matches!(kind, CardKind::Question | CardKind::Permission)
}

fn context(text: &str) -> Value {
    json!({ "type": "context", "elements": [{ "type": "mrkdwn", "text": text }] })
}

fn label(text: &str) -> String {
    let chars: Vec<char> = text.trim().chars().collect();
    if chars.len() <= LABEL_CHARS {
        return chars.into_iter().collect();
    }
    let mut kept: String = chars[..LABEL_CHARS - 1].iter().collect();
    kept.push('\u{2026}');
    kept
}

fn fallback(card: &Card) -> String {
    let text = cut_to(&card.text, FALLBACK_CHARS).0;
    let line = match (&card.from, &card.settled) {
        (_, Some(settled)) if text.is_empty() => settled.clone(),
        (Some(from), _) => format!("{from}: {text}"),
        (None, _) => text,
    };
    escape(&line)
}

pub fn sections(text: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut count = 0;
    for line in text.split('\n') {
        let mut line: Vec<char> = line.chars().collect();
        while line.len() > SECTION_CHARS {
            if count > 0 {
                parts.push(std::mem::take(&mut current));
                count = 0;
            }
            let cut = line[..SECTION_CHARS]
                .iter()
                .rposition(|c| c.is_whitespace())
                .filter(|space| *space > SECTION_CHARS / 2)
                .map_or(SECTION_CHARS, |space| space + 1);
            parts.push(line.drain(..cut).collect());
        }
        let needed = line.len() + usize::from(count > 0);
        if count > 0 && count + needed > SECTION_CHARS {
            parts.push(std::mem::take(&mut current));
            count = 0;
        }
        if count > 0 {
            current.push('\n');
            count += 1;
        }
        current.extend(line.iter());
        count += line.len();
    }
    if !current.trim().is_empty() {
        parts.push(current);
    }
    parts.retain(|part| !part.trim().is_empty());
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::place::Choice;

    fn card(kind: CardKind) -> Card {
        Card {
            kind,
            from: Some("Pip".into()),
            text: "Ship **it**?".into(),
            fields: vec![],
            ask: Some("ap_1".into()),
            choices: vec![
                Choice { id: "allow".into(), label: "Allow".into(), style: ChoiceStyle::Primary },
                Choice {
                    id: "alwaysAllow".into(),
                    label: "Always allow".into(),
                    style: ChoiceStyle::Plain,
                },
                Choice { id: "deny".into(), label: "Deny".into(), style: ChoiceStyle::Danger },
            ],
            settled: None,
        }
    }

    #[test]
    fn a_permission_renders_its_details_and_one_button_per_choice() {
        let mut permission = card(CardKind::Permission);
        permission.fields = vec![("Action".into(), "Create agent <Rex>".into())];
        let (blocks, fallback) = render(&permission);
        assert_eq!(blocks[0]["type"], "context");
        assert_eq!(blocks[0]["elements"][0]["text"], "*Pip*");
        assert_eq!(blocks[1]["text"]["text"], "Ship *it*?");
        assert_eq!(blocks[2]["fields"][0]["text"], "*Action*\nCreate agent &lt;Rex&gt;");
        let buttons = blocks[3]["elements"].as_array().unwrap();
        assert_eq!(buttons.len(), 3);
        assert_eq!(buttons[0]["action_id"], "ask:ap_1:allow");
        assert_eq!(buttons[0]["style"], "primary");
        assert!(buttons[1].get("style").is_none());
        assert_eq!(buttons[2]["style"], "danger");
        assert_eq!(fallback, "Pip: Ship **it**?");
    }

    #[test]
    fn a_settled_ask_trades_its_buttons_for_a_line() {
        let mut question = card(CardKind::Question);
        question.settled = Some("Answered: Allow".into());
        let (blocks, _) = render(&question);
        assert!(blocks.iter().all(|block| block["type"] != "actions"));
        assert_eq!(blocks.last().unwrap()["elements"][0]["text"], "Answered: Allow");
    }

    #[test]
    fn a_reply_or_notice_never_shows_buttons() {
        for kind in [CardKind::Reply, CardKind::Notice] {
            let (blocks, _) = render(&card(kind));
            assert!(blocks.iter().all(|block| block["type"] != "actions"), "{kind:?}");
        }
        let mut notice = card(CardKind::Notice);
        notice.from = None;
        let (blocks, fallback) = render(&notice);
        assert_eq!(blocks[0]["type"], "section");
        assert_eq!(fallback, "Ship **it**?");
    }

    #[test]
    fn labels_and_sections_stay_inside_slacks_limits() {
        assert_eq!(label(&"x".repeat(LABEL_CHARS)).chars().count(), LABEL_CHARS);
        assert_eq!(label(&"y".repeat(LABEL_CHARS + 9)).chars().count(), LABEL_CHARS);
        let long = format!("{}\n{}\n{}", "a".repeat(2000), "b".repeat(2000), "c".repeat(7000));
        let parts = sections(&long);
        assert!(parts.iter().all(|part| part.chars().count() <= SECTION_CHARS));
        assert_eq!(parts.concat().replace('\n', ""), long.replace('\n', ""));
        assert!(sections("").is_empty());
        let words = "word ".repeat(1000);
        let parts = sections(&words);
        assert!(parts.iter().all(|part| part.ends_with(' ')), "a long line is cut between words");
        assert_eq!(parts.concat(), words);
        let mut blank = card(CardKind::Question);
        blank.choices[0].label = "  ".into();
        assert_eq!(render(&blank).0.last().unwrap()["elements"][0]["text"]["text"], "allow");
    }

    #[test]
    fn an_action_id_survives_the_round_trip() {
        assert_eq!(parse_action(&action_id("ap:7", "0")), Some(("ap:7".into(), "0".into())));
        assert_eq!(parse_action("open_modal"), None);
        assert_eq!(parse_action("ask::x"), None);
    }
}
