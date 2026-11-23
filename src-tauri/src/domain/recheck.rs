use chrono::{Datelike, Duration, NaiveDate, Weekday};

use super::calculator::day_name;

pub const RECENT_DAYS: i64 = 31;

const SEPARATORS: &[char] = &[' ', ',', '.', '(', '*', '_', '\u{a0}'];

const WEEKDAYS: &[(&str, Weekday)] = &[
    ("monday", Weekday::Mon),
    ("mon", Weekday::Mon),
    ("tuesday", Weekday::Tue),
    ("tues", Weekday::Tue),
    ("tue", Weekday::Tue),
    ("wednesday", Weekday::Wed),
    ("wed", Weekday::Wed),
    ("thursday", Weekday::Thu),
    ("thurs", Weekday::Thu),
    ("thur", Weekday::Thu),
    ("thu", Weekday::Thu),
    ("friday", Weekday::Fri),
    ("fri", Weekday::Fri),
    ("saturday", Weekday::Sat),
    ("sat", Weekday::Sat),
    ("sunday", Weekday::Sun),
    ("sun", Weekday::Sun),
];

const MONTHS: &[&str] = &[
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrongDay {
    pub said: String,
    pub date: NaiveDate,
    pub stated: Weekday,
}

impl WrongDay {
    pub fn correction(&self) -> String {
        let back = (1..7)
            .map(|days| self.date - Duration::days(days))
            .find(|day| day.weekday() == self.stated)
            .unwrap_or(self.date);
        let ahead = (1..7)
            .map(|days| self.date + Duration::days(days))
            .find(|day| day.weekday() == self.stated)
            .unwrap_or(self.date);
        let stated = day_name(self.stated);
        format!(
            "\"{}\": {} is a {}, not a {stated}. The {stated}s either side of it are {} and {}.",
            self.said,
            self.date.format("%-d %B %Y"),
            day_name(self.date.weekday()),
            back.format("%-d %B %Y"),
            ahead.format("%-d %B %Y"),
        )
    }
}

#[derive(Debug, Clone, Copy)]
struct Word<'a> {
    text: &'a str,
    start: usize,
    end: usize,
}

pub fn without_code(text: &str) -> String {
    let mut kept = String::with_capacity(text.len());
    let mut fenced = false;
    for line in text.split_inclusive('\n') {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            kept.push('\n');
            continue;
        }
        if fenced {
            kept.push('\n');
            continue;
        }
        let mut inline = false;
        for c in line.chars() {
            if c == '`' {
                inline = !inline;
                kept.push(' ');
            } else if inline && c != '\n' {
                kept.push(' ');
            } else {
                kept.push(c);
            }
        }
    }
    kept
}

fn words(text: &str) -> Vec<Word<'_>> {
    let mut found = Vec::new();
    let mut start = None;
    for (at, c) in text.char_indices() {
        match (c.is_alphanumeric(), start) {
            (true, None) => start = Some(at),
            (false, Some(from)) => {
                found.push(Word { text: &text[from..at], start: from, end: at });
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        found.push(Word { text: &text[from..], start: from, end: text.len() });
    }
    found
}

fn capitalized(word: &str) -> bool {
    word.chars().next().is_some_and(char::is_uppercase)
}

fn weekday(word: &str) -> Option<Weekday> {
    if !capitalized(word) {
        return None;
    }
    let lower = word.to_lowercase();
    WEEKDAYS.iter().find(|(name, _)| *name == lower).map(|(_, day)| *day)
}

fn month(word: &str) -> Option<u32> {
    if !capitalized(word) {
        return None;
    }
    let lower = word.to_lowercase();
    let at = MONTHS.iter().position(|name| {
        *name == lower || (lower.len() >= 3 && name.starts_with(&lower) && lower.len() <= 4)
    })?;
    Some(at as u32 + 1)
}

fn day_of_month(word: &str) -> Option<u32> {
    let digits = word.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let suffix = &word[digits.len()..];
    if !(1..=2).contains(&digits.len())
        || !digits.chars().all(|c| c.is_ascii_digit())
        || !matches!(suffix.to_lowercase().as_str(), "" | "st" | "nd" | "rd" | "th")
    {
        return None;
    }
    digits.parse().ok().filter(|day| (1..=31).contains(day))
}

fn year(word: &str) -> Option<i32> {
    (word.len() == 4 && word.chars().all(|c| c.is_ascii_digit()))
        .then(|| word.parse().ok())
        .flatten()
        .filter(|year| (1900..=2200).contains(year))
}

struct Reader<'a> {
    text: &'a str,
    words: Vec<Word<'a>>,
}

impl<'a> Reader<'a> {
    fn joined(&self, at: usize) -> bool {
        at > 0
            && self.text[self.words[at - 1].end..self.words[at].start]
                .chars()
                .all(|c| SEPARATORS.contains(&c))
    }

    fn next(&self, at: usize) -> Option<usize> {
        (at + 1 < self.words.len() && self.joined(at + 1)).then_some(at + 1)
    }

    fn is(&self, at: usize, word: &str) -> bool {
        self.words[at].text.eq_ignore_ascii_case(word)
    }

    fn date_at(&self, at: usize) -> Option<(u32, u32, Option<i32>, usize)> {
        let mut at = at;
        if self.is(at, "the") {
            at = self.next(at)?;
        }
        if let Some(found) = self.iso_at(at) {
            return Some(found);
        }
        let first = self.words[at].text;
        let (month_number, day, last) = if let Some(day) = day_of_month(first) {
            let mut then = self.next(at)?;
            if self.is(then, "of") {
                then = self.next(then)?;
            }
            (month(self.words[then].text)?, day, then)
        } else {
            let month_number = month(first)?;
            let mut then = self.next(at)?;
            if self.is(then, "the") {
                then = self.next(then)?;
            }
            (month_number, day_of_month(self.words[then].text)?, then)
        };
        match self.next(last).and_then(|then| year(self.words[then].text).map(|y| (y, then))) {
            Some((stated, then)) => Some((month_number, day, Some(stated), then)),
            None => Some((month_number, day, None, last)),
        }
    }

    fn iso_at(&self, at: usize) -> Option<(u32, u32, Option<i32>, usize)> {
        let stated = year(self.words[at].text)?;
        let parts = [at + 1, at + 2];
        if parts.iter().any(|part| *part >= self.words.len()) {
            return None;
        }
        for part in parts {
            let gap = &self.text[self.words[part - 1].end..self.words[part].start];
            let word = self.words[part].text;
            if gap != "-" || word.len() != 2 || !word.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
        }
        let month_number = self.words[at + 1].text.parse().ok()?;
        let day = self.words[at + 2].text.parse().ok()?;
        Some((month_number, day, Some(stated), at + 2))
    }
}

fn resolve(month: u32, day: u32, stated: Option<i32>, today: NaiveDate) -> Option<NaiveDate> {
    if let Some(year) = stated {
        return NaiveDate::from_ymd_opt(year, month, day);
    }
    let earliest = today - Duration::days(RECENT_DAYS);
    (today.year()..=today.year() + 8)
        .filter_map(|year| NaiveDate::from_ymd_opt(year, month, day))
        .find(|date| *date >= earliest)
}

pub fn wrong_weekdays(text: &str, today: NaiveDate) -> Vec<WrongDay> {
    let plain = without_code(text);
    let reader = Reader { text: &plain, words: words(&plain) };
    let mut wrong: Vec<WrongDay> = Vec::new();
    let mut at = 0;
    while at < reader.words.len() {
        let mut paired = None;
        if let Some(stated) = weekday(reader.words[at].text) {
            if let Some(then) = reader.next(at) {
                if let Some((m, d, y, last)) = reader.date_at(then) {
                    paired = Some((stated, m, d, y, at, last));
                }
            }
        } else if let Some((m, d, y, last)) = reader.date_at(at) {
            let bracketed = reader.next(last).filter(|then| {
                plain[reader.words[last].end..reader.words[*then].start].contains('(')
            });
            if let Some(then) = bracketed {
                if let Some(stated) = weekday(reader.words[then].text) {
                    paired = Some((stated, m, d, y, at, then));
                }
            }
        }
        match paired {
            Some((stated, m, d, y, first, last)) => {
                if let Some(date) = resolve(m, d, y, today) {
                    let said = plain[reader.words[first].start..reader.words[last].end].to_string();
                    if date.weekday() != stated && !wrong.iter().any(|one| one.said == said) {
                        wrong.push(WrongDay { said, date, stated });
                    }
                }
                at = last + 1;
            }
            None => at += 1,
        }
    }
    wrong
}

#[derive(Debug, Clone, PartialEq)]
pub struct Figure {
    pub said: String,
    pub value: f64,
    pub step: f64,
    pub percent: bool,
}

fn number_at(text: &str, from: usize) -> Option<(f64, f64, usize)> {
    let bytes = text.as_bytes();
    let mut end = from;
    let mut decimals: Option<u32> = None;
    let mut digits = String::new();
    while let Some(&c) = bytes.get(end) {
        let grouped = c == b','
            && decimals.is_none()
            && bytes
                .get(end + 1..end + 4)
                .is_some_and(|group| group.iter().all(u8::is_ascii_digit))
            && !bytes.get(end + 4).is_some_and(u8::is_ascii_digit);
        let point =
            c == b'.' && decimals.is_none() && bytes.get(end + 1).is_some_and(u8::is_ascii_digit);
        if c.is_ascii_digit() {
            digits.push(c as char);
            if let Some(count) = decimals.as_mut() {
                *count += 1;
            }
        } else if point {
            digits.push('.');
            decimals = Some(0);
        } else if !grouped {
            break;
        }
        end += 1;
    }
    if digits.is_empty() || digits == "." {
        return None;
    }
    let value: f64 = digits.parse().ok()?;
    let step = match decimals {
        Some(count) => 10f64.powi(-(count as i32)),
        None => {
            let zeros = digits.chars().rev().take_while(|c| *c == '0').count();
            if zeros == digits.len() {
                1.0
            } else {
                10f64.powi(zeros as i32)
            }
        }
    };
    Some((value, step, end))
}

fn scale_at(text: &str, from: usize) -> (f64, usize) {
    let rest = &text[from..];
    let trimmed = rest.trim_start_matches(' ');
    let gap = rest.len() - trimmed.len();
    let lower = trimmed.to_lowercase();
    for (word, scale) in [("million", 1e6), ("thousand", 1e3), ("k", 1e3), ("m", 1e6)] {
        let Some(after) = lower.strip_prefix(word) else { continue };
        let close = !after.chars().next().is_some_and(char::is_alphanumeric);
        if close && (word.len() > 1 || gap == 0) {
            return (scale, from + gap + word.len());
        }
    }
    (1.0, from)
}

pub fn figures_in(text: &str) -> Vec<Figure> {
    let plain = without_code(text);
    let mut found = Vec::new();
    let bytes = plain.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'$' {
            let mut from = at + 1;
            while bytes.get(from) == Some(&b' ') {
                from += 1;
            }
            if let Some((value, step, end)) = number_at(&plain, from) {
                let (scale, end) = scale_at(&plain, end);
                found.push(Figure {
                    said: plain[at..end].to_string(),
                    value: value * scale,
                    step: step * scale,
                    percent: false,
                });
                at = end;
                continue;
            }
        } else if bytes[at].is_ascii_digit() && (at == 0 || !is_number_byte(bytes[at - 1])) {
            if let Some((value, step, end)) = number_at(&plain, at) {
                let rest = &plain[end..];
                let percent = if rest.starts_with('%') {
                    Some(end + 1)
                } else if rest.to_lowercase().starts_with(" percent") {
                    Some(end + " percent".len())
                } else {
                    None
                };
                if let Some(close) = percent {
                    found.push(Figure {
                        said: plain[at..close].to_string(),
                        value,
                        step,
                        percent: true,
                    });
                }
                at = end.max(at + 1);
                continue;
            }
        }
        at += 1;
    }
    found
}

fn is_number_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b',' | b'$')
}

pub fn numbers_in(text: &str) -> Vec<f64> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let starts = bytes[at].is_ascii_digit()
            && (at == 0 || !(bytes[at - 1].is_ascii_digit() || bytes[at - 1] == b'.'));
        if let Some((value, _, end)) = starts.then(|| number_at(text, at)).flatten() {
            let (scale, _) = scale_at(text, end);
            found.push(value);
            if scale != 1.0 {
                found.push(value * scale);
            }
            at = end.max(at + 1);
            continue;
        }
        at += 1;
    }
    found
}

impl Figure {
    pub fn matches(&self, known: f64) -> bool {
        let known = known.abs();
        let near = |value: f64| (value - self.value).abs() <= self.step / 2.0 + 1e-9 * value.abs();
        near(known) || (self.percent && near(known * 100.0))
    }

    fn trivial(&self) -> bool {
        self.value == 0.0 || (self.percent && self.value == 100.0)
    }
}

pub fn unchecked_figures(answer: &str, known: &[f64]) -> Vec<String> {
    let mut left: Vec<String> = Vec::new();
    for figure in figures_in(answer) {
        if figure.trivial() || known.iter().any(|n| figure.matches(*n)) {
            continue;
        }
        if !left.contains(&figure.said) {
            left.push(figure.said);
        }
    }
    left
}

pub fn send_back(wrong: &[WrongDay], unchecked: &[String]) -> Option<String> {
    if wrong.is_empty() && unchecked.is_empty() {
        return None;
    }
    let mut said = String::from(
        "Before this reaches the operator, it was checked, and it needs correcting. Write the \
         whole answer again with the correction made; the operator will see only the new one.\n",
    );
    if !wrong.is_empty() {
        said.push_str("\nWeekdays that are wrong:\n");
        for one in wrong {
            said.push_str(&format!("- {}\n", one.correction()));
        }
        let first = wrong.iter().map(|one| one.date).min().unwrap_or_default();
        let last = wrong.iter().map(|one| one.date).max().unwrap_or_default();
        let weeks = ((last - first).num_days() / 7 + 3).min(super::occasion::WEEKS_AHEAD);
        said.push_str(&format!(
            "Fix the plan, not only the word: anything you planned around that day, such as a \
             weekend, a day off or how far away it is, has to follow the true weekday, and so \
             does every other weekday you write. The weeks around it, each line Monday to \
             Sunday:\n{}",
            super::occasion::weeks_of(first - Duration::days(7), weeks)
        ));
    }
    if !unchecked.is_empty() {
        said.push_str(&format!(
            "\nThese figures are neither in what you were given nor in anything `calculator` \
             returned this turn: {}.\nWork out each one you derived with `calculator` and correct \
             the answer to what it returns. A figure you chose as an assumption may stay, if the \
             answer says it is an assumption.\n",
            unchecked.join(", ")
        ));
    }
    Some(said)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 23).unwrap()
    }

    fn wrong(text: &str) -> Vec<String> {
        wrong_weekdays(text, today()).iter().map(WrongDay::correction).collect()
    }

    #[test]
    fn a_wrong_weekday_is_found_in_every_shape_a_date_is_written() {
        for said in [
            "Saturday, 20 December 2026",
            "Saturday 20 December",
            "Saturday, December 20, 2026",
            "Saturday, December 20th",
            "Sat Dec 20",
            "Sat, 20 Dec",
            "Sat. Dec. 20",
            "Saturday the 20th of December",
            "Saturday (Dec 20)",
            "**Saturday, Dec 20**",
            "Saturday 2026-12-20",
            "20 December 2026 (Saturday)",
            "Dec 20 (Sat)",
        ] {
            let text = format!("The cleanup is on {said}, from 9 to noon.");
            let found = wrong_weekdays(&text, today());
            assert_eq!(found.len(), 1, "{said}: {found:?}");
            assert_eq!(found[0].date, NaiveDate::from_ymd_opt(2026, 12, 20).unwrap(), "{said}");
            assert_eq!(found[0].stated, Weekday::Sat, "{said}");
        }
        assert_eq!(
            wrong("Hold it on Saturday Dec 6."),
            vec!["\"Saturday Dec 6\": 6 December 2026 is a Sunday, not a Saturday. The Saturdays \
                 either side of it are 5 December 2026 and 12 December 2026."
                .to_string()]
        );
    }

    #[test]
    fn a_right_weekday_passes_in_every_shape() {
        for said in [
            "Sunday, 20 December 2026",
            "Sun Dec 20",
            "Sunday the 20th of December",
            "Saturday, December 19",
            "Sat 5 Dec",
            "Friday, 1 January 2027",
            "Tuesday, 29 February 2028",
            "Thu Jan 7",
        ] {
            assert!(wrong(&format!("It is on {said}.")).is_empty(), "{said}");
        }
    }

    #[test]
    fn a_missing_year_is_the_next_such_date_and_a_recent_one_is_this_year() {
        assert!(wrong("Friday, 1 January").is_empty());
        assert!(wrong("Monday, 16 November").is_empty());
        assert_eq!(wrong("Thursday, 1 January").len(), 1);
    }

    #[test]
    fn a_date_or_a_weekday_alone_is_not_a_claim() {
        assert!(wrong("Book the park for 20 December 2026.").is_empty());
        assert!(wrong("Meet on Saturday morning at the park.").is_empty());
        assert!(wrong("Saturday\n20 December: the cleanup.").is_empty());
        assert!(wrong("The sun 20 December rose late.").is_empty());
        assert!(wrong("Sat 20 Dec is in a table | Sat | 21 Dec |").len() == 1);
        assert!(wrong("Saturday may 3 people come").is_empty());
        assert!(wrong("On Saturday, 45 people came in December.").is_empty());
    }

    #[test]
    fn a_date_inside_code_is_left_alone() {
        assert!(wrong("Run this:\n```\nSaturday, 20 December 2026\n```\nDone.").is_empty());
        assert!(wrong("The log says `Sat Dec 20` there.").is_empty());
        assert_eq!(wrong("```\nx\n```\nSaturday, 20 December 2026").len(), 1);
    }

    #[test]
    fn money_and_percentages_are_read_with_their_precision() {
        let found =
            figures_in("Buy costs $19,626 and $676.65 a month at 6%, or $18k, 2.5 percent.");
        let shown: Vec<(&str, f64, f64)> =
            found.iter().map(|f| (f.said.as_str(), f.value, f.step)).collect();
        assert_eq!(
            shown,
            vec![
                ("$19,626", 19626.0, 1.0),
                ("$676.65", 676.65, 0.01),
                ("6%", 6.0, 1.0),
                ("$18k", 18000.0, 1000.0),
                ("2.5 percent", 2.5, 0.1),
            ]
        );
        assert!(figures_in("In 2026, 30 volunteers and 3 years.").is_empty());
        assert!(figures_in("```\n$500\n```").is_empty());
    }

    #[test]
    fn a_figure_given_or_returned_passes_however_it_was_rounded_or_written() {
        let given = numbers_in("It costs $35,000, borrow at 6% over five years, $500 a month.");
        let returned =
            numbers_in("buy = 19626.1234\npayment = 676.648037\nr = 0.005\nlease = -1626");
        let known: Vec<f64> = given.into_iter().chain(returned).collect();
        let answer = "Buying costs $19,626, or $19,626.12, at $676.65 a month: 6% a year, 0.5% a \
                      month, on $35,000. Leasing is $1,626 cheaper than that, about $19,600.";
        assert_eq!(unchecked_figures(answer, &known), Vec::<String>::new());
    }

    #[test]
    fn a_figure_nobody_worked_out_is_named() {
        let known = numbers_in("buy = 19626.12\nlease = 21000\n18626");
        assert_eq!(
            unchecked_figures(
                "Break-even is $18,253. At $17,000 buying saves $107. Buying costs $19,626.",
                &known
            ),
            vec!["$18,253".to_string(), "$17,000".to_string(), "$107".to_string()]
        );
        assert_eq!(
            unchecked_figures("Nothing down: $0, 100% financed.", &[]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_send_back_names_each_fault_and_nothing_when_there_is_none() {
        assert_eq!(send_back(&[], &[]), None);
        let wrong = wrong_weekdays("Saturday, 20 December 2026", today());
        let said = send_back(&wrong, &["$18,253".to_string()]).unwrap();
        assert!(said.contains("20 December 2026 is a Sunday, not a Saturday"), "{said}");
        assert!(
            said.contains(
                "The Saturdays either side of it are 19 December 2026 and 26 December 2026."
            ),
            "{said}"
        );
        assert!(said.contains("- Mon 14 Dec, Tue 15 Dec"), "the week before it: {said}");
        assert!(said.contains("Sat 19 Dec, Sun 20 Dec 2026"), "its own week: {said}");
        assert!(said.contains("Sat 26 Dec, Sun 27 Dec 2026"), "the week after: {said}");
        assert!(said.contains("Fix the plan, not only the word"), "{said}");
        assert!(said.contains("$18,253"), "{said}");
        assert!(said.contains("as an assumption"), "{said}");
        let days_only = send_back(&wrong, &[]).unwrap();
        assert!(!days_only.contains("calculator"), "{days_only}");
    }
}
