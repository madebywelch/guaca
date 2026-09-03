//! An occasion: a date the crew is answerable for.
//!
//! Guaca's own calendar, and not a view of anybody else's. The Google plugin
//! reads the operator's real calendar and is the right tool for what is already
//! on it; this is the other half, which nothing else in the app holds. An agent
//! that learns a filing is due on the 15th, that a customer moved a call, or
//! that a contract lapses at the end of the month has nowhere to put it: memory
//! is what the agent knows and outlives the date, a working note expires on its
//! own and carries no moment, and a routine is work the agent will do rather
//! than a thing that is happening. All three were reached for and all three
//! were wrong, because none of them can be read as "what is coming".
//!
//! So an occasion is exactly one fact: something is happening, at a time, and
//! the crew is answerable for it. It runs nothing, fires nothing and wakes
//! nobody. A routine is what you set when the date should *do* something, and
//! the two are deliberately not folded together: a calendar that fires turns
//! every note about a customer's schedule into an agent waking up at 3am.
//!
//! ## Whose calendar it is
//!
//! A crew's, and that is the load-bearing part rather than a filing decision.
//! Every mutation an agent makes is resolved inside the group on its own card,
//! never against an id it supplied: an occasion in another crew's calendar is
//! not refused, it is *not found*, because refusing tells an agent that a row
//! it may not touch exists and who it belongs to. Same wall the message bus
//! keeps, and it has to be kept here for a reason the bus does not have: an id
//! is a thing a model can invent, and an invented one that happened to land on
//! another crew's board meeting would move it.
//!
//! The operator stands above that wall, which is why the commands they call
//! take an id and no group. It is their workspace and all of it is theirs to
//! read at once; the wall is between crews, not between them and a crew.
//!
//! ## What is not here
//!
//! No recurrence. "Every Monday at ten" is four words to say and a field that
//! turns one row into a series with exceptions, a rewrite rule and a question
//! about what "cancel" means. Nothing has needed it yet, and the honest way to
//! keep a standing meeting on this calendar today is a routine that writes the
//! next one.
//!
//! No invitations, no attendees, no reply state. Nobody outside this machine
//! can see this calendar, so there is nobody to invite.

use chrono::{Duration, NaiveDate, NaiveTime};
use serde::{Deserialize, Serialize};

use super::cut_to;
use super::ids::{AgentId, GroupId, OccasionId};
use super::{instant, local};

/// How long a title may be. A line in a day's list, not a description.
///
/// What runs past it belongs in `detail`, and the cut is handed back rather
/// than applied silently for the reason every other cut in this app is: an
/// agent that believes it wrote something it did not will not write it again.
pub const MAX_TITLE: usize = 120;

/// The note under it: what the operator needs in order to walk in prepared.
///
/// Bigger than a working note and much smaller than a document. An agent given
/// room for a briefing writes one, and a calendar of briefings is a calendar
/// nobody scans.
pub const MAX_DETAIL: usize = 600;

/// Where it happens. A room, a city, a link.
pub const MAX_PLACE: usize = 200;

/// The longest an occasion may run, in minutes: thirty days.
///
/// Not a judgment about long events. It is the cap that keeps a model's unit
/// slip from drawing a bar across the year: `minutes: 90000` for "an hour and a
/// half in seconds" is the mistake this catches, and it is caught loudly
/// because a silently accepted one is a row the operator has to work out.
pub const MAX_MINUTES: u32 = 60 * 24 * 30;

/// How far ahead an agent is shown its crew's calendar, in days.
///
/// Two weeks is what "what is coming" means to somebody planning: far enough
/// that a filing on the 15th is visible on the 3rd, near enough that the list
/// is not a year of quarterly obligations. Anything past it is one `list` call
/// away, and the prompt says so.
pub const HORIZON_DAYS: i64 = 14;

/// How many the prompt draws, however busy the fortnight is.
///
/// A cap rather than a scroll: this list is on every turn of every agent in the
/// crew, and a crew that fills it is a crew whose next turn is mostly calendar.
pub const MAX_SHOWN: usize = 12;

/// One occasion, as it is stored and as it is read back.
///
/// Flat rather than an enum over the two shapes a date can have, and the
/// invariant that costs is kept in exactly one place: [`Clean::new`] is the
/// only way to build one for writing, and it drops `minutes` on an all-day
/// occasion. A day has no length to state, and a row carrying both would draw
/// "all day, 30 minutes".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Occasion {
    pub id: OccasionId,
    /// The crew whose calendar this is on. Never changes: moving an occasion
    /// between crews is deleting it from one and writing it on the other, and
    /// there is no surface that offers it.
    pub group_id: GroupId,
    /// Who put it there. `None` is the operator, and it is also what is left
    /// when the agent that wrote it has been deleted for good: the date does
    /// not stop being true because the agent that noticed it is gone.
    pub agent_id: Option<AgentId>,
    pub title: String,
    /// What the operator needs to know to walk into it. Often empty.
    pub detail: String,
    /// A room, a city, a link. Often empty.
    pub place: String,
    /// When it starts, as an instant. Local midnight of the day for an all-day
    /// one, which is what lets one index and one `ORDER BY` serve both kinds.
    pub starts_at: i64,
    /// How long it runs. `None` is a moment with no stated end, which is most
    /// of them: "the contract lapses" has no duration and inventing one would
    /// draw a bar for a thing that is not a meeting.
    pub minutes: Option<u32>,
    /// A day with no time on it: a deadline, a birthday, a filing. Distinct
    /// from midnight, which is a real time somebody chose.
    pub all_day: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Occasion {
    /// When it is over, for one that says. `None` for a moment and for a day.
    pub fn ends_at(&self) -> Option<i64> {
        self.minutes.map(|minutes| self.starts_at + i64::from(minutes) * 60_000)
    }

    /// The whole of it in one line, the way it is said out loud.
    ///
    /// Used everywhere an agent reads its crew's calendar: the prompt, the
    /// answer to `list`, and the confirmation after a write. One rendering, so
    /// an agent that adds something is told it back in the words it will see it
    /// in next turn — which is the check that catches a date read wrong far
    /// faster than any refusal could.
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

/// A validated occasion, ready to be written. The only way to make one.
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
    /// Whether anything the writer sent was cut to fit. Handed back to them.
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
    /// Checks what was asked for and settles the one invariant the row has.
    ///
    /// `minutes` is dropped rather than refused on an all-day occasion. A model
    /// that sends both means the day, and refusing the call would cost a round
    /// trip to be told something nobody would disagree with.
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
            // The whole invariant, in the one place a row is built.
            minutes: if when.all_day { None } else { minutes },
            all_day: when.all_day,
            cut: title_cut || detail_cut || place_cut,
        })
    }
}

/// A moment, and whether anybody put a time on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct When {
    pub starts_at: i64,
    pub all_day: bool,
}

/// Reads the date a model or an operator wrote.
///
/// Local time, always, and stated in the tool description as such. What is
/// accepted is what people and models actually write:
///
/// - `2026-09-14` — a day, with no time on it.
/// - `2026-09-14 15:00`, `2026-09-14T15:00`, with optional `:ss` — a local
///   moment. The `T` is ISO 8601 and the space is what everything else writes.
/// - either of those with `Z` or `+02:00` on the end — an instant somebody
///   stated the zone of, converted to this machine's local time.
///
/// The last case is accepted rather than refused, and that is a decision worth
/// naming: a model told "local, no zone" writes one anyway, and a refusal costs
/// a round trip to relearn a rule it already had. An explicit zone is not
/// ambiguous, so it is honored. What makes that safe is not the parser — it is
/// that every writer is answered with [`Occasion::describe`], which restates
/// the local wall clock that was stored, so a model that meant 3pm and got 11am
/// reads it in the same turn.
pub fn parse_when(text: &str) -> Result<When, Invalid> {
    let raw = text.trim();
    let bad = || Invalid::NoDate(raw.to_string());
    if raw.is_empty() {
        return Err(bad());
    }

    // A stated zone first: it is unambiguous, and nothing below it could tell
    // `+02:00` from a malformed local time.
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
        // A day with no time on it. Local midnight, so one column orders the
        // two kinds against each other and a day still sorts before the
        // meetings on it.
        let midnight = date.and_time(NaiveTime::MIN);
        return Ok(When { starts_at: instant(midnight).ok_or_else(bad)?, all_day: true });
    };

    let clock = NaiveTime::parse_from_str(time, "%H:%M:%S")
        .or_else(|_| NaiveTime::parse_from_str(time, "%H:%M"))
        // Written the way it is said, which a model does often enough that
        // refusing it would be refusing a correct date over its formatting.
        .or_else(|_| NaiveTime::parse_from_str(&spoken(time), "%I:%M %p"))
        .map_err(|_| bad())?;

    Ok(When { starts_at: instant(date.and_time(clock)).ok_or_else(bad)?, all_day: false })
}

/// `9am` as `9:00 AM`, which is the one form chrono's `%p` reads.
///
/// Three normalizations, and each is a way a person writes a time rather than a
/// format anybody chose: the meridiem is matched against `AM`/`PM` and nothing
/// else, it may be written against the hour with no space, and an hour on its
/// own has no minute for `NaiveTime` to be built from. A model writes all three
/// often enough that refusing them would be refusing correct dates over their
/// spelling.
fn spoken(time: &str) -> String {
    let raw = time.to_uppercase().replace(' ', "");
    let Some(hour) = raw.strip_suffix("AM").or_else(|| raw.strip_suffix("PM")) else {
        return raw;
    };
    let meridiem = &raw[hour.len()..];
    let hour = if hour.contains(':') { hour.to_string() } else { format!("{hour}:00") };
    format!("{hour} {meridiem}")
}

/// The local day a moment falls on, as midnight. What the operator's view
/// groups by and what the horizon is measured from.
pub fn day_of(at: i64) -> i64 {
    local(at)
        .and_then(|when| instant(when.date_naive().and_time(NaiveTime::MIN)))
        // A timestamp no calendar can hold is its own answer: there is no day
        // to round it to, so it stays where it is rather than becoming zero.
        .unwrap_or(at)
}

/// The end of the window an agent is shown, counted in whole local days so a
/// fortnight is a fortnight either side of a clock change.
pub fn horizon(now: i64) -> i64 {
    local(now)
        .and_then(|when| when.date_naive().checked_add_signed(Duration::days(HORIZON_DAYS + 1)))
        .and_then(|date| instant(date.and_time(NaiveTime::MIN)))
        .unwrap_or(now + Duration::days(HORIZON_DAYS + 1).num_milliseconds())
}

/// When it is, as a person says it: `Wed 14 Sep, 3:00 PM`, or `Wed 14 Sep`.
///
/// Weekday first because that is what the question is. An operator reading a
/// calendar wants to know it is a Thursday long before they want the number,
/// and a model deciding whether it can promise a call on Tuesday needs the same
/// thing without doing date arithmetic to get it.
pub fn when_words(at: i64, all_day: bool) -> String {
    let Some(when) = local(at) else { return "an unreadable date".to_string() };
    let day = when.format("%a %-d %b %Y");
    if all_day {
        format!("{day}, all day")
    } else {
        format!("{day}, {}", when.format("%-I:%M %p"))
    }
}

/// How far off it is, in the coarsest unit that is still true.
///
/// The same argument `worknote::how_long_ago` makes, pointed the other way. An
/// agent handed an ISO timestamp and a clock has to do arithmetic against both
/// and gets it wrong; handed "in 3 days" it has the answer it was going to
/// compute. Both are drawn, because the absolute one is what a person acts on
/// and the relative one is what a model reasons with.
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

/// A duration as it is said: `30m`, `1h`, `1h 30m`, `2 days`.
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

/// Whether two occasions land on the same clock time. Used to warn a writer
/// that a crew already has something then, never to refuse the write.
pub fn overlaps(one: &Occasion, other: &Occasion) -> bool {
    // An all-day occasion is not a clash with anything. A filing deadline and a
    // 3pm call on the same day are both true, and a warning about it would fire
    // on every date anybody put on the calendar.
    if one.all_day || other.all_day {
        return false;
    }
    let one_end = one.ends_at().unwrap_or(one.starts_at);
    let other_end = other.ends_at().unwrap_or(other.starts_at);
    one.starts_at <= other_end && other.starts_at <= one_end
}

/// A local wall-clock moment, as a timestamp. Test-only, and the same helper
/// `routine.rs` keeps for the same reason: every assertion below is written in
/// wall-clock terms and has to survive being run in any zone.
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
        // A model asked for `HH:MM` sends `3:00 PM` often enough that refusing
        // it would be refusing a correct date over its formatting.
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
        // The safety here is not the parser. It is that every writer is
        // answered with the local wall clock that was stored.
        let when = parse_when("2026-09-14T15:00:00Z").unwrap();
        assert!(!when.all_day);
        // Built through a different door than the parser under test, so this
        // is a comparison rather than a restatement.
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
        // Not the 1st of October. A model that means the 30th and writes the
        // 31st has to be told, because a silently moved date is one nobody
        // checks.
        assert!(parse_when("2026-09-31").is_err());
        assert!(parse_when("2026-09-14 25:00").is_err());
    }

    #[test]
    fn an_all_day_occasion_has_no_length_however_it_was_asked_for() {
        // The one invariant the row has, settled where a row is built rather
        // than trusted to every caller. A model that sends both means the day.
        let day = clean("Quarterly filing", parse_when("2026-09-14").unwrap(), Some(30)).unwrap();
        assert!(day.all_day);
        assert_eq!(day.minutes, None);
    }

    #[test]
    fn a_zero_length_is_a_moment_rather_than_a_zero() {
        // A model padding out its arguments writes `minutes: 0`, which as a
        // stored value would draw a bar of no width.
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
        // 90000 is "an hour and a half" counted in seconds. Accepted, it is a
        // sixty-two day bar the operator has to work out.
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
        // The absolute half is what a person acts on; the relative half is what
        // a model reasons with instead of doing arithmetic and getting it
        // wrong. Neither one alone is enough.
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
        // 11pm tonight and 1am tomorrow are two hours apart and are not the
        // same answer to "when is it".
        let now = at((2026, 9, 12), (23, 0));
        assert_eq!(how_far(at((2026, 9, 12), (23, 30)), now), "today");
        assert_eq!(how_far(at((2026, 9, 13), (1, 0)), now), "tomorrow");
    }

    #[test]
    fn something_that_has_already_happened_today_says_so() {
        // It is still on today's list and must not read as though it is coming.
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
        // Counted in days rather than in milliseconds, so a clock change does
        // not shorten or lengthen the window an agent is shown.
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

        // A filing due the same day is not a conflict with a call, and warning
        // about it would fire on every date anybody wrote down.
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
