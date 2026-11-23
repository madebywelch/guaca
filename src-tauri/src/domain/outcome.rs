use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

use super::cut_to;
use super::envelope::{Envelope, Part, Participant};
use super::ids::{AgentId, DecisionId, EscalationId, GroupId, MessageId, RunId};

pub const HEADLINE_MAX: usize = 160;

pub const NO_NOTE: &str = "Finished without a note.";

pub const STOPPED: &str = "Stopped before it finished.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settled {
    Finished,
    Stopped,
    Absorbed,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Trigger {
    Operator,
    Routine,
    Event,
    Decision,
    Retry,
}

impl Trigger {
    pub fn of(start: &Envelope) -> Option<(Trigger, Option<String>)> {
        if let Some(original) = start.cause {
            return Some((Trigger::Retry, Some(original.to_string())));
        }
        for part in &start.parts {
            match part {
                Part::Routine { routine_id, payload, .. } => {
                    let trigger = if payload.is_some() { Trigger::Event } else { Trigger::Routine };
                    return Some((trigger, Some(routine_id.to_string())));
                }
                Part::Decision { id, .. } => {
                    return Some((Trigger::Decision, Some(id.to_string())));
                }
                _ => {}
            }
        }
        (start.from == Participant::Human).then_some((Trigger::Operator, None))
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Trigger::Operator => "operator",
            Trigger::Routine => "routine",
            Trigger::Event => "event",
            Trigger::Decision => "decision",
            Trigger::Retry => "retry",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "operator" => Trigger::Operator,
            "routine" => Trigger::Routine,
            "event" => Trigger::Event,
            "decision" => Trigger::Decision,
            "retry" => Trigger::Retry,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Done,
    NothingNew,
    NeedsYou,
    Failed,
    Stopped,
    Interrupted,
}

impl Status {
    pub const fn as_str(self) -> &'static str {
        match self {
            Status::Done => "done",
            Status::NothingNew => "nothing_new",
            Status::NeedsYou => "needs_you",
            Status::Failed => "failed",
            Status::Stopped => "stopped",
            Status::Interrupted => "interrupted",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "done" => Status::Done,
            "nothing_new" => Status::NothingNew,
            "needs_you" => Status::NeedsYou,
            "failed" => Status::Failed,
            "stopped" => Status::Stopped,
            "interrupted" => Status::Interrupted,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Need {
    Decision { id: DecisionId },
    Escalation { id: EscalationId },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Running {
    pub run_id: RunId,
    pub agent_id: AgentId,
    pub group_id: GroupId,
    pub started_at: i64,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub run_id: RunId,
    pub agent_id: AgentId,
    pub group_id: GroupId,
    pub trigger: Trigger,
    pub trigger_ref: Option<String>,
    pub status: Status,
    pub headline: String,
    pub note_id: Option<MessageId>,
    pub colleagues: Vec<AgentId>,
    pub needs: Vec<Need>,
    pub steps: u32,
    pub started_at: i64,
    pub finished_at: i64,
    pub seen_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRecord {
    pub messages: Option<Vec<Envelope>>,
    pub result_id: Option<MessageId>,
}

pub const ELLIPSIS: char = '…';

pub fn headline(note: &str) -> Option<String> {
    first_line(note).map(|words| shorten(&words))
}

fn shorten(line: &str) -> String {
    let line = line.trim();
    if line.chars().count() <= HEADLINE_MAX {
        return line.to_string();
    }
    if let Some(end) = sentence_end(line) {
        return format!("{}{ELLIPSIS}", &line[..end]);
    }
    let mut chars = 0;
    let mut end = 0;
    for (at, segment) in line.split_word_bound_indices() {
        chars += segment.chars().count();
        if chars >= HEADLINE_MAX {
            break;
        }
        end = at + segment.len();
    }
    format!("{}{ELLIPSIS}", line[..end].trim_end())
}

fn sentence_end(line: &str) -> Option<usize> {
    let mut last = None;
    let mut chars = line.char_indices().enumerate().peekable();
    while let Some((count, (at, c))) = chars.next() {
        if count >= HEADLINE_MAX - 1 {
            break;
        }
        let closes = match c {
            '.' | '!' | '?' => chars.peek().is_some_and(|(_, (_, next))| next.is_whitespace()),
            '。' | '！' | '？' => true,
            _ => false,
        };
        if closes && count >= HEADLINE_MAX / 2 {
            last = Some(at + c.len_utf8());
        }
    }
    last
}

pub const BOTTOM_LINE: &str = "Bottom line";

pub fn first_line(note: &str) -> Option<String> {
    let lines = lines(note);
    if let Some(answer) = bottom_line(&lines) {
        return Some(answer);
    }
    let mut worded = lines.iter().enumerate().filter(|(_, line)| !line.words.is_empty()).peekable();
    while let Some((at, line)) = worded.next() {
        let Some(&(after, next)) = worded.peek() else {
            return Some(line.words.clone());
        };
        if is_label(&line.words) || (line.title && lines.get(at + 1).is_some_and(|l| l.rule)) {
            continue;
        }
        if let Some(kept) = lead_in(&line.words) {
            if kept.is_empty() {
                continue;
            }
            return Some(kept.to_string());
        }
        if after == at + 1 {
            if line.words.ends_with(',') {
                return Some(format!("{} {}", line.words, next.words));
            }
            if line.words.ends_with(['，', '、']) {
                return Some(format!("{}{}", line.words, next.words));
            }
        }
        return Some(line.words.clone());
    }
    None
}

fn bottom_line(lines: &[Line]) -> Option<String> {
    lines.iter().enumerate().rev().find_map(|(at, line)| {
        let words = line.words.get(..BOTTOM_LINE.len())?;
        if !words.eq_ignore_ascii_case(BOTTOM_LINE) {
            return None;
        }
        let rest = line.words[BOTTOM_LINE.len()..].trim_start();
        let answer = rest.strip_prefix(':').or_else(|| rest.strip_prefix('：'))?.trim();
        if !answer.is_empty() {
            return Some(answer.to_string());
        }
        lines.get(at + 1).filter(|next| !next.words.is_empty()).map(|next| next.words.clone())
    })
}

struct Line {
    words: String,
    title: bool,
    rule: bool,
}

fn lines(note: &str) -> Vec<Line> {
    let mut fenced = false;
    let mut out = Vec::new();
    for line in note.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        if is_rule(trimmed) {
            out.push(Line { words: String::new(), title: false, rule: true });
            continue;
        }
        let words = plain(trimmed);
        if !words.is_empty() {
            out.push(Line { words, title: is_title(trimmed), rule: false });
        }
    }
    out
}

fn is_title(line: &str) -> bool {
    heading(line).is_some()
        || ["**", "__"].iter().any(|mark| {
            line.len() > 2 * mark.len() && line.starts_with(mark) && line.ends_with(mark)
        })
}

fn is_label(words: &str) -> bool {
    words.unicode_words().count() == 1 && words.chars().all(char::is_alphanumeric)
}

fn lead_in(words: &str) -> Option<&str> {
    let open = ["...", "…", ":", "："].iter().find_map(|end| words.strip_suffix(end))?;
    let mut end = 0;
    let mut chars = open.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        let closes = match c {
            '.' | '!' | '?' => chars.peek().is_some_and(|(_, next)| next.is_whitespace()),
            '。' | '！' | '？' => true,
            _ => false,
        };
        if closes {
            end = at + c.len_utf8();
        }
    }
    Some(&open[..end])
}

pub fn headline_of(note: &Envelope) -> Option<String> {
    if let Some(words) = headline(&note.plain_text()) {
        return Some(words);
    }
    let names: Vec<&str> = note
        .parts
        .iter()
        .filter_map(|part| match part {
            Part::File(file) => Some(file.name.as_str()),
            _ => None,
        })
        .collect();
    let first = names.first()?;
    let line = match names.len() {
        1 => format!("Sent {first}."),
        2 => format!("Sent {first} and 1 other file."),
        n => format!("Sent {first} and {} other files.", n - 1),
    };
    Some(shorten(&line))
}

pub fn first_sentence(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or_default();
    let mut end = line.len();
    let mut chars = line.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        let closes = match c {
            '.' | '!' | '?' => chars.peek().is_none_or(|(_, next)| next.is_whitespace()),
            '。' | '！' | '？' => true,
            _ => false,
        };
        if closes {
            end = at + c.len_utf8();
            break;
        }
    }
    cut_to(&line[..end], HEADLINE_MAX).0
}

fn plain(line: &str) -> String {
    if is_rule(line) {
        return String::new();
    }
    let mut rest = line;
    loop {
        let before = rest;
        rest = rest.trim_start();
        if let Some(after) = rest.strip_prefix('>') {
            rest = after;
        } else if let Some(after) = heading(rest) {
            rest = after;
        } else if let Some(after) = list_item(rest) {
            rest = after;
        } else if let Some(after) = marker(rest) {
            rest = after;
        }
        if rest == before {
            break;
        }
    }
    inline(rest).split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_rule(line: &str) -> bool {
    let marks = line.chars().filter(|c| !c.is_whitespace()).count();
    marks >= 3 && line.chars().all(|c| matches!(c, '-' | '*' | '_' | '=' | '|' | ':' | ' ' | '\t'))
}

fn heading(line: &str) -> Option<&str> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let after = &line[hashes..];
    (after.is_empty() || after.starts_with(char::is_whitespace)).then_some(after)
}

fn list_item(line: &str) -> Option<&str> {
    for bullet in ["- ", "* ", "+ "] {
        if let Some(after) = line.strip_prefix(bullet) {
            return Some(after);
        }
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if (1..=9).contains(&digits) {
        let after = &line[digits..];
        for closer in [". ", ") "] {
            if let Some(after) = after.strip_prefix(closer) {
                return Some(after);
            }
        }
    }
    for task in ["[ ] ", "[x] ", "[X] "] {
        if let Some(after) = line.strip_prefix(task) {
            return Some(after);
        }
    }
    None
}

fn marker(line: &str) -> Option<&str> {
    let inner = line.strip_prefix("[!")?;
    let close = inner.find(']')?;
    let word = &inner[..close];
    (!word.is_empty() && word.chars().all(|c| c.is_ascii_alphabetic())).then(|| &inner[close + 1..])
}

fn inline(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '!' && chars.get(i + 1) == Some(&'[') {
            i += 1;
            continue;
        }
        if c == '[' {
            if let Some((label, next)) = link(&chars, i) {
                out.push_str(&inline(&label));
                i = next;
                continue;
            }
        }
        if c == '`' || (c == '~' && chars.get(i + 1) == Some(&'~')) {
            i += if c == '~' { 2 } else { 1 };
            continue;
        }
        if c == '*' || c == '_' {
            let before = i.checked_sub(1).map(|at| chars[at]);
            let mut run = i;
            while chars.get(run) == Some(&c) {
                run += 1;
            }
            let after = chars.get(run).copied();
            let inside_word = before.is_some_and(char::is_alphanumeric)
                && after.is_some_and(char::is_alphanumeric);
            let standing_alone =
                before.is_none_or(char::is_whitespace) && after.is_none_or(char::is_whitespace);
            if inside_word || standing_alone {
                out.extend(&chars[i..run]);
            }
            i = run;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

fn link(chars: &[char], open: usize) -> Option<(String, usize)> {
    let close = open + chars[open..].iter().position(|c| *c == ']')?;
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = close + 1 + chars[close + 1..].iter().position(|c| *c == ')')?;
    Some((chars[open + 1..close].iter().collect(), end + 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::envelope::{Intent, Trust};
    use crate::domain::ids::RoutineId;

    #[test]
    fn a_plain_first_line_is_the_headline() {
        assert_eq!(
            headline("Sent the digest to 12 people.\n\nDetails below.").as_deref(),
            Some("Sent the digest to 12 people.")
        );
    }

    #[test]
    fn a_label_over_a_draft_is_passed_over_for_the_draft() {
        let draft =
            "Hi team,\nStarting next week, our weekly meeting moves from Monday to Thursday \
                     morning at [time].\n\nSee you there!";
        let opening = "Hi team, Starting next week, our weekly meeting moves from Monday to \
                       Thursday morning at [time].";
        assert_eq!(headline(&format!("Draft\n\n{draft}")).as_deref(), Some(opening));
        assert_eq!(headline(&format!("**Draft**\n\n{draft}")).as_deref(), Some(opening));
        assert_eq!(
            headline(
                "**Draft team note**\n\n---\n\nHi everyone,\n\nJust a quick heads-up: starting \
                 next week, our weekly meeting is moving from Monday to Thursday morning at \
                 [time].\n\nLooking forward to seeing you all then!\n\n[Your sign-off]\n\n---"
            )
            .as_deref(),
            Some(
                "Hi everyone, Just a quick heads-up: starting next week, our weekly meeting is \
                 moving from Monday to Thursday morning at [time]."
            )
        );
        assert_eq!(
            headline(
                "**Draft note for your team about the meeting change.**\n\n---\n\nHey \
                 everyone,\n\nWe meet on Thursdays from next week."
            )
            .as_deref(),
            Some("Hey everyone, We meet on Thursdays from next week.")
        );
    }

    #[test]
    fn a_line_that_only_leads_into_what_follows_is_passed_over() {
        let answer = "Buy: it costs $17,621 over three years against $21,000 to lease.";
        let rest = format!("{answer}\n\n## Buying with the loan\n\n- Monthly payment: $676.65");
        assert_eq!(
            headline(&format!("I'll work this out for you...\n\n{rest}")).as_deref(),
            Some(answer)
        );
        assert_eq!(
            headline(&format!("I'll work this out for you…\n\n{rest}")).as_deref(),
            Some(answer)
        );
        assert_eq!(
            headline(
                "Here's your draft:\n\n---\n\n**Subject: Weekly team meeting moving to Thursday \
                 mornings**\n\nHi everyone,"
            )
            .as_deref(),
            Some("Subject: Weekly team meeting moving to Thursday mornings")
        );
        assert_eq!(
            headline("## Risks:\n\n1. No waiver signed").as_deref(),
            Some("No waiver signed")
        );
        assert_eq!(
            headline("結論：\n\nリースより購入が安い。").as_deref(),
            Some("リースより購入が安い。")
        );
    }

    #[test]
    fn a_sentence_before_a_lead_in_is_kept_without_it() {
        assert_eq!(
            headline(
                "Buy it: you come out $3,379 ahead. Here's the arithmetic:\n\n- Loan: $676.65"
            )
            .as_deref(),
            Some("Buy it: you come out $3,379 ahead.")
        );
    }

    #[test]
    fn a_question_after_its_lead_in_is_the_question() {
        assert_eq!(
            headline(
                "I need to know:\n- What's the neighborhood you have in mind?\n- Which Saturday \
                 or Sunday works?"
            )
            .as_deref(),
            Some("What's the neighborhood you have in mind?")
        );
    }

    #[test]
    fn an_answer_with_nothing_before_it_reads_as_it_did() {
        assert_eq!(
            headline(
                "Lease, if you replace the car within three years.\n\n**Lease:** $21,000\n\n---\n\n\
                 ## What would change it\n\nA lower rate."
            )
            .as_deref(),
            Some("Lease, if you replace the car within three years.")
        );
        assert_eq!(headline("Done.\n\nSent the digest to 12 people.").as_deref(), Some("Done."));
        assert_eq!(headline("Draft").as_deref(), Some("Draft"));
        assert_eq!(
            headline("Here is the query:\n```sql\nselect 1\n```").as_deref(),
            Some("Here is the query:")
        );
        assert_eq!(
            headline("毎朝のダイジェストを十二人に送信しました\n\n詳細は下に").as_deref(),
            Some("毎朝のダイジェストを十二人に送信しました")
        );
        assert_eq!(
            headline("Revenue is up 4%, costs up 6%\n\n## Detail").as_deref(),
            Some("Revenue is up 4%, costs up 6%")
        );
    }

    #[test]
    fn the_first_line_is_what_the_headline_cuts_and_is_never_cut_itself() {
        let long = format!("**Done:** {}", "every feed checked and nothing missed, ".repeat(6));
        let line = first_line(&format!("\n> [!NOTE]\n> {long}\n\nDetail.")).unwrap();
        assert!(line.starts_with("Done: every feed checked"), "{line}");
        assert!(line.chars().count() > HEADLINE_MAX, "{}", line.chars().count());
        assert_eq!(headline(&long).unwrap(), shorten(&line));
        assert_eq!(first_line("```\nonly code\n```"), None);
    }

    #[test]
    fn a_heading_is_read_without_its_hashes() {
        assert_eq!(
            headline("## Triage done: 2 replies drafted\n\n- one\n- two").as_deref(),
            Some("Triage done: 2 replies drafted")
        );
        assert_eq!(headline("#\n# Nightly check").as_deref(), Some("Nightly check"));
        assert_eq!(headline("#hashtag is a word").as_deref(), Some("#hashtag is a word"));
    }

    #[test]
    fn a_callout_marker_is_the_box_and_not_the_words() {
        assert_eq!(
            headline("> [!IMPORTANT]\n> The API key expires Friday. Rotate it.").as_deref(),
            Some("The API key expires Friday. Rotate it.")
        );
        assert_eq!(
            headline("> [!NOTE] Nothing changed overnight.").as_deref(),
            Some("Nothing changed overnight.")
        );
    }

    #[test]
    fn a_list_item_is_read_as_its_words() {
        assert_eq!(
            headline("- **Booked** the room for Tuesday").as_deref(),
            Some("Booked the room for Tuesday")
        );
        assert_eq!(headline("1. Merged the two drafts").as_deref(), Some("Merged the two drafts"));
        assert_eq!(
            headline("* [x] Filed the expense report").as_deref(),
            Some("Filed the expense report")
        );
    }

    #[test]
    fn blank_lines_and_rules_before_the_words_are_passed_over() {
        assert_eq!(
            headline("\n\n   \n---\nChecked all four feeds.").as_deref(),
            Some("Checked all four feeds.")
        );
    }

    #[test]
    fn emphasis_code_and_links_keep_their_words() {
        assert_eq!(
            headline(
                "Updated `pending_runs` and _closed_ [the ticket](https://example.com/1) in \
                 ~~two~~ *one* pass, 2 * 3 times"
            )
            .as_deref(),
            Some("Updated pending_runs and closed the ticket in two one pass, 2 * 3 times")
        );
    }

    const WORKED: &str = "I'll work through both options.\n\n## Buying\n\n- Payment: $676.65 a \
                          month\n- Still owed after 36 months: $15,267\n\n## Leasing\n\n$3,000 + \
                          36 x $500 = $21,000";

    #[test]
    fn a_bottom_line_is_the_headline_without_its_label() {
        let note = format!("{WORKED}\n\nBottom line: Buy, at $19,626 against $21,000 to lease.");
        assert_eq!(headline(&note).as_deref(), Some("Buy, at $19,626 against $21,000 to lease."));
        let bold = format!("{WORKED}\n\n**Bottom line:** Buy, at $19,626.");
        assert_eq!(headline(&bold).as_deref(), Some("Buy, at $19,626."));
        let outside = format!("{WORKED}\n\n**Bottom line**: Buy.");
        assert_eq!(headline(&outside).as_deref(), Some("Buy."));
        let below = format!("{WORKED}\n\n**Bottom line:**\nBuy, at $19,626.");
        assert_eq!(headline(&below).as_deref(), Some("Buy, at $19,626."));
    }

    #[test]
    fn a_result_with_no_bottom_line_is_read_as_before() {
        assert_eq!(headline(WORKED).as_deref(), Some("I'll work through both options."));
        let mentioned = "The bottom line is that buying wins.\n\nBottom lines: none.";
        assert_eq!(headline(mentioned).as_deref(), Some("The bottom line is that buying wins."));
    }

    #[test]
    fn of_two_bottom_lines_the_last_is_the_headline() {
        let note = "Bottom line: Lease, on a first look.\n\nThen the balance.\n\nBottom line: Buy.";
        assert_eq!(headline(note).as_deref(), Some("Buy."));
    }

    #[test]
    fn a_bottom_line_with_nothing_after_its_label_is_passed_over() {
        let note = "Buy: it costs $19,626 against $21,000.\n\nBottom line:";
        assert_eq!(headline(note).as_deref(), Some("Buy: it costs $19,626 against $21,000."));
        let blank = "Buy: it costs $19,626 against $21,000.\n\nBottom line:   \n\n---";
        assert_eq!(headline(blank).as_deref(), Some("Buy: it costs $19,626 against $21,000."));
    }

    #[test]
    fn a_bottom_line_inside_a_code_block_is_not_the_headline() {
        let note = "Buy: it costs $19,626.\n\n```\nBottom line: print(answer)\n```";
        assert_eq!(headline(note).as_deref(), Some("Buy: it costs $19,626."));
        let both = "Buy.\n\nBottom line: Buy the car.\n\n~~~\nBottom line: not this\n~~~";
        assert_eq!(headline(both).as_deref(), Some("Buy the car."));
    }

    #[test]
    fn a_long_bottom_line_is_cut_like_any_headline() {
        let long = format!("Bottom line: {}", "buy the car and keep it ".repeat(12));
        let shown = headline(&long).unwrap();
        assert!(shown.ends_with(ELLIPSIS) && shown.starts_with("buy the car"), "{shown}");
        assert!(shown.chars().count() <= HEADLINE_MAX, "{shown}");
    }

    #[test]
    fn a_fence_is_not_a_headline() {
        assert_eq!(
            headline("```chart\n{\"type\":\"bar\"}\n```\nRevenue is up 4%.").as_deref(),
            Some("Revenue is up 4%.")
        );
        assert_eq!(headline("```\nls -la\n```"), None);
        assert_eq!(headline(""), None);
        assert_eq!(headline("> \n---\n"), None);
    }

    #[test]
    fn a_long_first_line_is_cut_on_a_word_at_the_limit() {
        let line = "Reviewed every open pull request in the repository and left comments on the \
                    eleven that touch the scheduler, the two that change the budget and the one \
                    that renames the outbox table";
        assert!(line.chars().count() > HEADLINE_MAX);
        let cut = headline(line).unwrap();
        assert!(cut.chars().count() <= HEADLINE_MAX, "{} characters", cut.chars().count());
        let kept = cut.strip_suffix(ELLIPSIS).expect("a cut headline ends in an ellipsis");
        assert!(line.starts_with(kept));
        assert_eq!(line[kept.len()..].chars().next(), Some(' '), "cut inside a word: {cut:?}");
    }

    #[test]
    fn a_long_first_line_is_cut_after_its_last_whole_sentence_when_that_keeps_most_of_it() {
        let line = "Lease, if you replace the car within three years and keep it under twelve thousand miles a year. Market Researcher and Financial Analyst agree on the three-year cost and disagree on resale, which matters only past year four.";
        assert!(line.chars().count() > HEADLINE_MAX);
        let cut = headline(line).unwrap();
        let kept = cut.strip_suffix(ELLIPSIS).unwrap();
        assert!(kept.ends_with("a year."), "{cut:?}");
        assert!(line.starts_with(kept));
    }

    #[test]
    fn a_short_first_sentence_is_not_where_a_long_line_is_cut() {
        let line = "Lease, if you replace the car within three years. Market Researcher and Financial Analyst agree on the three-year cost and disagree on resale, which matters only past year four.";
        let cut = headline(line).unwrap();
        let kept = cut.strip_suffix(ELLIPSIS).expect("a cut headline ends in an ellipsis");
        assert!(kept.contains("Market Researcher"), "{cut:?}");
        assert_eq!(line[kept.len()..].chars().next(), Some(' '), "cut inside a word: {cut:?}");
        assert!(cut.chars().count() <= HEADLINE_MAX);
    }

    #[test]
    fn a_line_that_fits_is_never_marked_as_cut() {
        let line = "Lease, if you replace the car within three years.";
        assert_eq!(headline(line).as_deref(), Some(line));
    }

    #[test]
    fn a_word_that_cannot_fit_is_omitted_even_after_a_short_prefix() {
        let word = "a".repeat(HEADLINE_MAX + 20);
        assert_eq!(headline(&format!("Reference {word}")).as_deref(), Some("Reference…"));
        assert_eq!(headline(&word).as_deref(), Some("…"));
    }

    #[test]
    fn a_language_without_spaces_is_cut_on_a_sentence_or_a_unicode_word_boundary() {
        let sentences = "毎朝のダイジェストを十二人に送信しました。".repeat(12);
        let cut = headline(&sentences).unwrap();
        let kept = cut.strip_suffix(ELLIPSIS).unwrap();
        assert!(kept.ends_with('。') && sentences.starts_with(kept), "{cut:?}");
        let line = "毎朝のダイジェストを十二人に送信しました".repeat(12);
        let cut = headline(&line).unwrap();
        let kept = cut.strip_suffix(ELLIPSIS).unwrap();
        assert!(!kept.is_empty() && cut.chars().count() <= HEADLINE_MAX);
        assert!(line.split_word_bound_indices().any(|(at, _)| at == kept.len()));
    }

    #[test]
    fn a_note_that_is_only_a_file_says_what_it_sent() {
        let file = |name: &str| {
            Part::File(crate::domain::attachment::Attachment {
                digest: "0".repeat(64),
                name: name.into(),
                mime: "application/pdf".into(),
                bytes: 1,
            })
        };
        let note = |parts| envelope(Participant::Agent { id: AgentId::new() }, parts);
        assert_eq!(headline_of(&note(vec![file("brief.pdf")])).as_deref(), Some("Sent brief.pdf."));
        assert_eq!(
            headline_of(&note(vec![file("a.pdf"), file("b.pdf"), file("c.pdf")])).as_deref(),
            Some("Sent a.pdf and 2 other files.")
        );
        assert_eq!(
            headline_of(&note(vec![Part::text("Here is the brief."), file("brief.pdf")]))
                .as_deref(),
            Some("Here is the brief.")
        );
        assert_eq!(headline_of(&note(vec![])), None);
    }

    #[test]
    fn a_failure_is_read_to_the_end_of_its_first_sentence() {
        assert_eq!(
            first_sentence(
                "Researcher could not reply: the provider returned 503. Try again later.\n\nIt \
                 had made 2 calls."
            ),
            "Researcher could not reply: the provider returned 503."
        );
        assert_eq!(
            first_sentence("Fetched api.example.com and stopped"),
            "Fetched api.example.com and stopped"
        );
        assert_eq!(
            first_sentence("接続できませんでした。もう一度試してください。"),
            "接続できませんでした。"
        );
    }

    fn envelope(from: Participant, parts: Vec<Part>) -> Envelope {
        Envelope {
            id: MessageId::new(),
            run_id: RunId::new(),
            channel_id: AgentId::new(),
            from,
            to: Participant::Agent { id: AgentId::new() },
            parts,
            trust: Trust::Operator,
            hop: 0,
            expects_reply: true,
            intent: Intent::Work,
            cause: None,
            created_at: 0,
        }
    }

    #[test]
    fn the_first_message_says_what_started_the_run() {
        let routine = RoutineId::new();
        let firing = |payload: Option<String>| Part::Routine {
            routine_id: routine,
            name: "Digest".into(),
            what: "Send the digest.".into(),
            payload,
        };
        assert_eq!(
            Trigger::of(&envelope(Participant::Human, vec![Part::text("hi")])),
            Some((Trigger::Operator, None))
        );
        assert_eq!(
            Trigger::of(&envelope(Participant::System, vec![firing(None)])),
            Some((Trigger::Routine, Some(routine.to_string())))
        );
        assert_eq!(
            Trigger::of(&envelope(Participant::System, vec![firing(Some("{}".into()))])),
            Some((Trigger::Event, Some(routine.to_string())))
        );
        let original = MessageId::new();
        let again =
            Envelope { cause: Some(original), ..envelope(Participant::System, vec![firing(None)]) };
        assert_eq!(Trigger::of(&again), Some((Trigger::Retry, Some(original.to_string()))));
        assert_eq!(Trigger::of(&envelope(Participant::System, vec![Part::text("?")])), None);
    }

    #[test]
    fn stored_names_read_back_as_what_they_were() {
        for status in [
            Status::Done,
            Status::NothingNew,
            Status::NeedsYou,
            Status::Failed,
            Status::Stopped,
            Status::Interrupted,
        ] {
            assert_eq!(Status::parse(status.as_str()), Some(status));
        }
        for trigger in
            [Trigger::Operator, Trigger::Routine, Trigger::Event, Trigger::Decision, Trigger::Retry]
        {
            assert_eq!(Trigger::parse(trigger.as_str()), Some(trigger));
        }
        assert_eq!(serde_json::to_value(Status::NeedsYou).unwrap(), "needsYou");
        assert_eq!(
            serde_json::to_value(Need::Decision { id: DecisionId::new() }).unwrap()["kind"],
            "decision"
        );
    }
}
