use chrono::{Duration, NaiveDate, NaiveTime};
use serde::{Deserialize, Serialize};

use super::cut_to;
use super::ids::{AgentId, GroupId, OccasionId};
use super::{instant, local};

pub const MAX_TITLE: usize = 120;

pub const MAX_DETAIL: usize = 600;

pub const MAX_PLACE: usize = 200;

pub const MAX_MINUTES: u32 = 60 * 24 * 30;

pub const HORIZON_DAYS: i64 = 14;

pub const MAX_SHOWN: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Occasion {
    pub id: OccasionId,
    pub group_id: GroupId,
    pub agent_id: Option<AgentId>,
    pub title: String,
    pub detail: String,
    pub place: String,
    pub starts_at: i64,
    pub minutes: Option<u32>,
    pub all_day: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Occasion {
    pub fn ends_at(&self) -> Option<i64> {
        self.minutes.map(|minutes| self.starts_at + i64::from(minutes) * 60_000)
    }

    pub fn describe(&self, now: i64) -> String {
        let mut said = format!("{} — {}", when_words(self.starts_at, self.all_day), self.title);
        if let Some(minutes) = self.minutes {
            said.push_str(&format!(" ({})", human_minutes(minutes)));
        }
        if !self.place.is_empty() {
            said.push_str(&format!(" · {}", self.place));
        }
        said.push_str(&format!(" · {}", how_far(self.starts_at, now)));
        said
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clean {
    pub group_id: GroupId,
    pub agent_id: Option<AgentId>,
    pub title: String,
    pub detail: String,
    pub place: String,
    pub starts_at: i64,
    pub minutes: Option<u32>,
    pub all_day: bool,
    pub cut: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Invalid {
    #[error("an occasion needs a title: what is happening, in a few words")]
    NoTitle,
    #[error(
        "{0:?} is not a date this understands. Write it as `2026-09-14 15:00` for a time, or \
         `2026-09-14` for a whole day"
    )]
    NoDate(String),
    #[error("nothing is that long: an occasion runs at most {} days", MAX_MINUTES / (60 * 24))]
    TooLong,
}

impl Clean {
    pub fn new(
        group_id: GroupId,
        agent_id: Option<AgentId>,
        title: &str,
        detail: &str,
        place: &str,
        when: When,
        minutes: Option<u32>,
    ) -> Result<Self, Invalid> {
        let (title, title_cut) = cut_to(title, MAX_TITLE);
        if title.is_empty() {
            return Err(Invalid::NoTitle);
        }
        let (detail, detail_cut) = cut_to(detail, MAX_DETAIL);
        let (place, place_cut) = cut_to(place, MAX_PLACE);

        let minutes = match minutes {
            Some(0) | None => None,
            Some(minutes) if minutes > MAX_MINUTES => return Err(Invalid::TooLong),
            Some(minutes) => Some(minutes),
        };

        Ok(Clean {
            group_id,
            agent_id,
            title,
            detail,
            place,
            starts_at: when.starts_at,
            minutes: if when.all_day { None } else { minutes },
            all_day: when.all_day,
            cut: title_cut || detail_cut || place_cut,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct When {
    pub starts_at: i64,
    pub all_day: bool,
}

pub fn parse_when(text: &str) -> Result<When, Invalid> {
    let raw = text.trim();
    let bad = || Invalid::NoDate(raw.to_string());
    if raw.is_empty() {
        return Err(bad());
    }

    if let Ok(fixed) = chrono::DateTime::parse_from_rfc3339(raw) {
        return Ok(When { starts_at: fixed.timestamp_millis(), all_day: false });
    }

    let body = raw.replace('T', " ");
    let (date, time) = match body.split_once(' ') {
        Some((date, time)) => (date, Some(time.trim())),
        None => (body.as_str(), None),
    };

    let date = NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d").map_err(|_| bad())?;
    let Some(time) = time.filter(|time| !time.is_empty()) else {
        let midnight = date.and_time(NaiveTime::MIN);
        return Ok(When { starts_at: instant(midnight).ok_or_else(bad)?, all_day: true });
    };

    let clock = NaiveTime::parse_from_str(time, "%H:%M:%S")
        .or_else(|_| NaiveTime::parse_from_str(time, "%H:%M"))
        .or_else(|_| NaiveTime::parse_from_str(&spoken(time), "%I:%M %p"))
        .map_err(|_| bad())?;

    Ok(When { starts_at: instant(date.and_time(clock)).ok_or_else(bad)?, all_day: false })
}

fn spoken(time: &str) -> String {
    let raw = time.to_uppercase().replace(' ', "");
    let Some(hour) = raw.strip_suffix("AM").or_else(|| raw.strip_suffix("PM")) else {
        return raw;
    };
    let meridiem = &raw[hour.len()..];
    let hour = if hour.contains(':') { hour.to_string() } else { format!("{hour}:00") };
    format!("{hour} {meridiem}")
}

pub fn day_of(at: i64) -> i64 {
    local(at).and_then(|when| instant(when.date_naive().and_time(NaiveTime::MIN))).unwrap_or(at)
}

pub fn horizon(now: i64) -> i64 {
    local(now)
        .and_then(|when| when.date_naive().checked_add_signed(Duration::days(HORIZON_DAYS + 1)))
        .and_then(|date| instant(date.and_time(NaiveTime::MIN)))
        .unwrap_or(now + Duration::days(HORIZON_DAYS + 1).num_milliseconds())
}

pub fn when_words(at: i64, all_day: bool) -> String {
    let Some(when) = local(at) else { return "an unreadable date".to_string() };
    let day = when.format("%a %-d %b %Y");
    if all_day {
        format!("{day}, all day")
    } else {
        format!("{day}, {}", when.format("%-I:%M %p"))
    }
}

pub fn how_far(at: i64, now: i64) -> String {
    let today = day_of(now);
    let that_day = day_of(at);
    let days = (that_day - today) / 86_400_000;

    match days {
        0 if at < now => "earlier today".to_string(),
        0 => "today".to_string(),
        1 => "tomorrow".to_string(),
        -1 => "yesterday".to_string(),
        2..=13 => format!("in {days} days"),
        14..=59 => format!("in {} weeks", days / 7),
        60.. => format!("in {} months", days / 30),
        _ => format!("{} days ago", -days),
    }
}

pub fn human_minutes(minutes: u32) -> String {
    match minutes {
        0..=59 => format!("{minutes}m"),
        _ if minutes >= 60 * 24 && minutes.is_multiple_of(60 * 24) => {
            let days = minutes / (60 * 24);
            if days == 1 {
                "1 day".to_string()
            } else {
                format!("{days} days")
            }
        }
        _ if minutes.is_multiple_of(60) => format!("{}h", minutes / 60),
        _ => format!("{}h {}m", minutes / 60, minutes % 60),
    }
}

pub fn overlaps(one: &Occasion, other: &Occasion) -> bool {
    if one.all_day || other.all_day {
        return false;
    }
    let one_end = one.ends_at().unwrap_or(one.starts_at);
    let other_end = other.ends_at().unwrap_or(other.starts_at);
    one.starts_at <= other_end && other.starts_at <= one_end
}

#[cfg(test)]
fn at(date: (i32, u32, u32), time: (u32, u32)) -> i64 {
    let date = NaiveDate::from_ymd_opt(date.0, date.1, date.2).unwrap();
    let clock = NaiveTime::from_hms_opt(time.0, time.1, 0).unwrap();
    instant(date.and_time(clock)).unwrap()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn group() -> GroupId {
        GroupId::new()
    }

    fn clean(title: &str, when: When, minutes: Option<u32>) -> Result<Clean, Invalid> {
        Clean::new(group(), None, title, "", "", when, minutes)
    }

    #[test]
    fn a_date_with_a_time_on_it_is_a_local_moment() {
        let when = parse_when("2026-09-14 15:00").unwrap();
        assert!(!when.all_day);
        assert_eq!(when.starts_at, at((2026, 9, 14), (15, 0)));
    }

    #[test]
    fn the_iso_separator_and_a_space_are_the_same_date() {
        assert_eq!(
            parse_when("2026-09-14T15:00").unwrap(),
            parse_when("2026-09-14 15:00").unwrap()
        );
    }

    #[test]
    fn seconds_are_allowed_and_ignored_past_the_minute() {
        assert_eq!(
            parse_when("2026-09-14 15:00:30").unwrap().starts_at,
            at((2026, 9, 14), (15, 0)) + 30_000
        );
    }

    #[test]
    fn a_time_written_the_way_it_is_said_is_read() {
        assert_eq!(parse_when("2026-09-14 3:00 PM").unwrap().starts_at, at((2026, 9, 14), (15, 0)));
        assert_eq!(parse_when("2026-09-14 9am").unwrap().starts_at, at((2026, 9, 14), (9, 0)));
    }

    #[test]
    fn a_date_with_no_time_is_a_whole_day_at_local_midnight() {
        let when = parse_when("2026-09-14").unwrap();
        assert!(when.all_day);
        assert_eq!(when.starts_at, at((2026, 9, 14), (0, 0)));
    }

    #[test]
    fn a_stated_zone_is_honored_rather_than_refused() {
        let when = parse_when("2026-09-14T15:00:00Z").unwrap();
        assert!(!when.all_day);
        let stated = chrono::Utc.with_ymd_and_hms(2026, 9, 14, 15, 0, 0).unwrap();
        assert_eq!(when.starts_at, stated.timestamp_millis());
    }

    #[test]
    fn nonsense_is_refused_with_the_two_shapes_that_work() {
        let err = parse_when("next tuesday").unwrap_err();
        let said = err.to_string();
        assert!(said.contains("2026-09-14 15:00"), "{said}");
        assert!(said.contains("2026-09-14"), "{said}");
    }

    #[test]
    fn an_impossible_date_is_refused_rather_than_rolled_over() {
        assert!(parse_when("2026-09-31").is_err());
        assert!(parse_when("2026-09-14 25:00").is_err());
    }

    #[test]
    fn an_all_day_occasion_has_no_length_however_it_was_asked_for() {
        let day = clean("Quarterly filing", parse_when("2026-09-14").unwrap(), Some(30)).unwrap();
        assert!(day.all_day);
        assert_eq!(day.minutes, None);
    }

    #[test]
    fn a_zero_length_is_a_moment_rather_than_a_zero() {
        let one = clean("Contract lapses", parse_when("2026-09-14 09:00").unwrap(), Some(0));
        assert_eq!(one.unwrap().minutes, None);
    }

    #[test]
    fn an_occasion_needs_something_to_call_it() {
        assert_eq!(
            clean("   ", parse_when("2026-09-14").unwrap(), None).unwrap_err(),
            Invalid::NoTitle
        );
    }

    #[test]
    fn a_unit_slip_is_refused_rather_than_drawn_across_the_year() {
        assert_eq!(
            clean("Standup", parse_when("2026-09-14 09:00").unwrap(), Some(90_000)).unwrap_err(),
            Invalid::TooLong
        );
    }

    #[test]
    fn an_over_long_title_is_cut_and_says_so() {
        let long = "word ".repeat(100);
        let one = clean(&long, parse_when("2026-09-14").unwrap(), None).unwrap();
        assert!(one.cut);
        assert!(one.title.chars().count() <= MAX_TITLE);
    }

    fn occasion(starts_at: i64, all_day: bool, minutes: Option<u32>) -> Occasion {
        Occasion {
            id: OccasionId::new(),
            group_id: group(),
            agent_id: None,
            title: "Board call".to_string(),
            detail: String::new(),
            place: String::new(),
            starts_at,
            minutes,
            all_day,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn a_line_carries_the_wall_clock_and_the_distance_both() {
        let now = at((2026, 9, 12), (9, 0));
        let one = occasion(at((2026, 9, 14), (15, 0)), false, Some(30));
        let said = one.describe(now);
        assert!(said.contains("Mon 14 Sep 2026"), "{said}");
        assert!(said.contains("3:00 PM"), "{said}");
        assert!(said.contains("30m"), "{said}");
        assert!(said.contains("in 2 days"), "{said}");
    }

    #[test]
    fn a_whole_day_says_so_rather_than_claiming_midnight() {
        let now = at((2026, 9, 12), (9, 0));
        let said = occasion(at((2026, 9, 14), (0, 0)), true, None).describe(now);
        assert!(said.contains("all day"), "{said}");
        assert!(!said.contains("12:00"), "{said}");
    }

    #[test]
    fn the_distance_is_counted_in_days_rather_than_in_hours() {
        let now = at((2026, 9, 12), (23, 0));
        assert_eq!(how_far(at((2026, 9, 12), (23, 30)), now), "today");
        assert_eq!(how_far(at((2026, 9, 13), (1, 0)), now), "tomorrow");
    }

    #[test]
    fn something_that_has_already_happened_today_says_so() {
        let now = at((2026, 9, 12), (15, 0));
        assert_eq!(how_far(at((2026, 9, 12), (9, 0)), now), "earlier today");
    }

    #[test]
    fn the_unit_gets_coarser_as_the_date_gets_further_off() {
        let now = at((2026, 9, 12), (9, 0));
        assert_eq!(how_far(at((2026, 9, 20), (9, 0)), now), "in 8 days");
        assert_eq!(how_far(at((2026, 10, 12), (9, 0)), now), "in 4 weeks");
        assert_eq!(how_far(at((2027, 1, 12), (9, 0)), now), "in 4 months");
        assert_eq!(how_far(at((2026, 9, 5), (9, 0)), now), "7 days ago");
    }

    #[test]
    fn a_fortnight_is_a_fortnight_of_local_days() {
        let now = at((2026, 3, 1), (9, 0));
        let end = horizon(now);
        assert_eq!(end, at((2026, 3, 16), (0, 0)));
        assert!(end > now);
    }

    #[test]
    fn a_length_reads_the_way_it_is_said() {
        assert_eq!(human_minutes(30), "30m");
        assert_eq!(human_minutes(60), "1h");
        assert_eq!(human_minutes(90), "1h 30m");
        assert_eq!(human_minutes(60 * 24), "1 day");
        assert_eq!(human_minutes(60 * 48), "2 days");
    }

    #[test]
    fn two_meetings_at_one_time_clash_and_a_deadline_never_does() {
        let three = occasion(at((2026, 9, 14), (15, 0)), false, Some(60));
        let half_past = occasion(at((2026, 9, 14), (15, 30)), false, Some(30));
        let five = occasion(at((2026, 9, 14), (17, 0)), false, Some(30));
        assert!(overlaps(&three, &half_past));
        assert!(!overlaps(&three, &five));

        let filing = occasion(at((2026, 9, 14), (0, 0)), true, None);
        assert!(!overlaps(&three, &filing));
    }

    #[test]
    fn a_moment_with_no_length_still_clashes_with_itself() {
        let one = occasion(at((2026, 9, 14), (15, 0)), false, None);
        let other = occasion(at((2026, 9, 14), (15, 0)), false, Some(30));
        assert!(overlaps(&one, &other));
    }

    #[test]
    fn an_occasion_that_states_a_length_knows_when_it_is_over() {
        let one = occasion(at((2026, 9, 14), (15, 0)), false, Some(90));
        assert_eq!(one.ends_at(), Some(at((2026, 9, 14), (16, 30))));
        assert_eq!(occasion(at((2026, 9, 14), (15, 0)), false, None).ends_at(), None);
    }
}
