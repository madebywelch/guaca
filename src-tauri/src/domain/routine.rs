//! Routines: an agent's own schedule.
//!
//! An agent sets these for itself. "Check the listings every five hours" and
//! "wake me in an hour" are the same thing with and without a repeat, so both
//! are one row: what to do, and when it is next due.
//!
//! What is stored is the next due time rather than a running timer, so a
//! schedule survives a restart. Nothing has to be held in memory for a routine
//! to fire tomorrow.

use chrono::{
    DateTime, Datelike, Duration, Local, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Weekday,
};
use serde::{Deserialize, Serialize};

use super::ids::{AgentId, RoutineId, RunId};

/// What makes a routine fire.
///
/// Today every one of these is a clock, which is why the wire form is a string
/// and not a number: the trigger an operator will eventually want is "when a
/// Linear issue is assigned to me", and that has to be a new value in this
/// column rather than a new column.
///
/// `Daily`, `Weekly` and `Monthly` are deliberately not gaps in seconds even
/// though two of them nearly are. A day is 23 or 25 hours twice a year, and a
/// month is four different lengths, so a routine stored as a gap wanders off
/// the hour it was set for. What is stored is the shape of the repeat; the
/// hour it happens at comes from `next_run_at`.
/// One string, on the wire and in the database both. A derived form would give
/// the webview a tagged object and SQLite a string for the same fact, and the
/// two would drift the first time either gained a variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// Fires once and is done.
    Once,
    /// A fixed gap, in seconds. What the agent's own `schedule` tool sets, and
    /// what "every hour" in the UI becomes.
    Every(u32),
    /// Every day, at the time of day it is already set for.
    Daily,
    /// Monday to Friday, at the time of day it is already set for.
    Weekdays,
    /// The same weekday every week.
    Weekly,
    /// The same day of the month every month.
    Monthly,
}

impl Trigger {
    /// The stored form. Parsed back by [`Trigger::parse`].
    pub fn as_str(&self) -> String {
        match self {
            Trigger::Once => "once".to_string(),
            Trigger::Every(secs) => format!("every:{secs}"),
            Trigger::Daily => "daily".to_string(),
            Trigger::Weekdays => "weekdays".to_string(),
            Trigger::Weekly => "weekly".to_string(),
            Trigger::Monthly => "monthly".to_string(),
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "once" => Some(Trigger::Once),
            "daily" => Some(Trigger::Daily),
            "weekdays" => Some(Trigger::Weekdays),
            "weekly" => Some(Trigger::Weekly),
            "monthly" => Some(Trigger::Monthly),
            other => other.strip_prefix("every:")?.parse().ok().map(Trigger::Every),
        }
    }

    pub fn repeats(&self) -> bool {
        !matches!(self, Trigger::Once)
    }

    /// How it reads to whoever set it.
    pub fn describe(&self) -> String {
        match self {
            Trigger::Once => "once".to_string(),
            Trigger::Every(secs) => format!("every {}", human_gap(*secs)),
            Trigger::Daily => "every day".to_string(),
            Trigger::Weekdays => "every weekday".to_string(),
            Trigger::Weekly => "every week".to_string(),
            Trigger::Monthly => "every month".to_string(),
        }
    }

    /// Whether a moment is one this trigger would ever fire on.
    ///
    /// Only weekdays can say no. It exists so a first run the operator asked
    /// for on a Saturday moves to Monday instead of firing on a day the
    /// routine says it never runs.
    pub fn accepts(&self, at: i64) -> bool {
        match self {
            Trigger::Weekdays => match local(at) {
                Some(when) => !is_weekend(when.date_naive()),
                None => false,
            },
            _ => true,
        }
    }

    /// When a routine that has just fired is next due.
    ///
    /// `slot` is the time it was due, not the time it actually ran: a machine
    /// asleep through Tuesday and Wednesday should wake and fire once on
    /// Thursday, at the hour it was set for, rather than three times to catch
    /// up. `None` means it is finished and the row goes.
    pub fn next_after(&self, slot: i64, now: i64) -> Option<i64> {
        match self {
            Trigger::Once => None,
            // A gap is counted from when it ran rather than from when it was
            // due, for the same no-catch-up reason. There is no hour to hold
            // on to here, so there is nothing to anchor to.
            Trigger::Every(secs) => Some(now + i64::from(*secs) * 1000),
            _ => self.next_calendar_slot(slot, now),
        }
    }

    /// When a routine just set should first fire.
    ///
    /// A repeat with no stated start waits one whole interval, which is what
    /// "every weekday" means to the person who said it: not now and then every
    /// weekday. A stated start is honoured, except on a day this trigger never
    /// fires on.
    pub fn first_run(&self, now: i64, in_secs: Option<u32>) -> i64 {
        let asked = in_secs.map(|delay| now + i64::from(delay) * 1000);
        match (self, asked) {
            (Trigger::Once, _) => asked.unwrap_or(now),
            (_, Some(at)) if self.accepts(at) => at,
            (Trigger::Every(secs), None) => now + i64::from(*secs) * 1000,
            // A start on a day this never fires on, or none at all: the anchor
            // keeps the hour and the trigger picks the day.
            (_, asked) => {
                let anchor = asked.unwrap_or(now);
                self.next_after(anchor, now).unwrap_or(anchor)
            }
        }
    }

    /// The next slot at the anchor's time of day that is still ahead of `now`.
    fn next_calendar_slot(&self, slot: i64, now: i64) -> Option<i64> {
        let anchor = local(slot)?;
        let date = anchor.date_naive();
        let time = anchor.time();

        for step in 1..=MAX_STEPS {
            let Some(next) = self.nth_date(date, step) else { continue };
            // A slot that does not exist in local time is skipped rather than
            // abandoned; `instant` has already nudged it past the DST gap.
            let Some(at) = instant(next.and_time(time)) else { continue };
            if at > now {
                return Some(at);
            }
        }
        None
    }

    /// The `n`th date this trigger fires on after `anchor`.
    ///
    /// Counted from the anchor every time rather than from the previous answer,
    /// so a monthly routine set on the 31st is the 28th in February and the
    /// 31st again in March instead of walking backwards down the calendar.
    fn nth_date(&self, anchor: NaiveDate, n: i64) -> Option<NaiveDate> {
        match self {
            Trigger::Daily => anchor.checked_add_signed(Duration::days(n)),
            Trigger::Weekly => anchor.checked_add_signed(Duration::days(n * 7)),
            Trigger::Weekdays => nth_weekday_after(anchor, n),
            Trigger::Monthly => months_after(anchor, n),
            Trigger::Once | Trigger::Every(_) => None,
        }
    }
}

impl Serialize for Trigger {
    fn serialize<S: serde::Serializer>(&self, out: S) -> Result<S::Ok, S::Error> {
        out.serialize_str(&self.as_str())
    }
}

impl<'de> Deserialize<'de> for Trigger {
    fn deserialize<D: serde::Deserializer<'de>>(input: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(input)?;
        Trigger::parse(&raw)
            .ok_or_else(|| serde::de::Error::custom(format!("no trigger called {raw:?}")))
    }
}

/// How far ahead a calendar slot is searched for.
///
/// A step is one date addition, so this is cheap; eleven years of daily steps
/// is the worst case a machine left switched off could produce. Exhausting it
/// means the stored slot is nonsense rather than merely stale.
const MAX_STEPS: i64 = 4000;

fn is_weekend(date: NaiveDate) -> bool {
    matches!(date.weekday(), Weekday::Sat | Weekday::Sun)
}

fn nth_weekday_after(anchor: NaiveDate, n: i64) -> Option<NaiveDate> {
    let mut date = anchor;
    for _ in 0..n {
        loop {
            date = date.checked_add_signed(Duration::days(1))?;
            if !is_weekend(date) {
                break;
            }
        }
    }
    Some(date)
}

fn months_after(anchor: NaiveDate, n: i64) -> Option<NaiveDate> {
    let months = i64::from(anchor.year()) * 12 + i64::from(anchor.month0()) + n;
    let year = i32::try_from(months.div_euclid(12)).ok()?;
    let month = u32::try_from(months.rem_euclid(12)).ok()? + 1;
    // The 31st of a thirty-day month is its 30th, not the 1st of the next one.
    // Rolling over would move a monthly routine a day later every short month.
    let day = anchor.day().min(days_in_month(year, month)?);
    NaiveDate::from_ymd_opt(year, month, day)
}

fn days_in_month(year: i32, month: u32) -> Option<u32> {
    let (next_year, next_month) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    NaiveDate::from_ymd_opt(next_year, next_month, 1)?.pred_opt().map(|last| last.day())
}

/// The local wall-clock moment a timestamp lands on.
fn local(ms: i64) -> Option<DateTime<Local>> {
    DateTime::from_timestamp_millis(ms).map(|utc| utc.with_timezone(&Local))
}

/// The instant a local wall-clock time happens at.
///
/// Both DST edges are handled here rather than left to the caller. Springing
/// forward deletes an hour, so a routine anchored inside it has no time to fire
/// at that day and takes the next hour; falling back doubles one, and the first
/// pass is the one that was meant.
fn instant(naive: NaiveDateTime) -> Option<i64> {
    match Local.from_local_datetime(&naive) {
        LocalResult::Single(at) => Some(at.timestamp_millis()),
        LocalResult::Ambiguous(first, _) => Some(first.timestamp_millis()),
        LocalResult::None => {
            let shifted = naive.checked_add_signed(Duration::hours(1))?;
            Local.from_local_datetime(&shifted).earliest().map(|at| at.timestamp_millis())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Routine {
    pub id: RoutineId,
    pub agent_id: AgentId,
    /// What the operator calls it. Blank on anything an agent set for itself
    /// without naming it, in which case the instruction is the name.
    pub name: String,
    /// The instruction the agent gave itself, delivered when it fires.
    pub what: String,
    pub trigger: Trigger,
    /// Set up but not running. The wording, the schedule and the history all
    /// survive being switched off, which is what makes it different from
    /// deleting the thing.
    pub active: bool,
    pub next_run_at: i64,
    pub last_run_at: Option<i64>,
    pub created_at: i64,
}

/// Why a routine ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunKind {
    /// It came due.
    Scheduled,
    /// The operator pressed the button. Nothing about the schedule moved.
    Test,
}

impl RunKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RunKind::Scheduled => "scheduled",
            RunKind::Test => "test",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "scheduled" => Some(RunKind::Scheduled),
            "test" => Some(RunKind::Test),
            _ => None,
        }
    }
}

impl Serialize for RunKind {
    fn serialize<S: serde::Serializer>(&self, out: S) -> Result<S::Ok, S::Error> {
        out.serialize_str(self.as_str())
    }
}

/// One firing, as the operator reads it back.
///
/// `run_id` is the thread back to everything else the firing produced: the
/// messages in the channel and the model calls on the bill are filed under it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineRun {
    pub run_id: RunId,
    pub kind: RunKind,
    pub at: i64,
}

impl Routine {
    pub fn repeats(&self) -> bool {
        self.trigger.repeats()
    }

    /// What to call it in a list. Never empty.
    pub fn title(&self) -> &str {
        if self.name.trim().is_empty() {
            &self.what
        } else {
            &self.name
        }
    }

    /// When this should next fire, given that it just ran at `now`.
    pub fn after_running(&self, now: i64) -> Option<i64> {
        self.trigger.next_after(self.next_run_at, now)
    }

    /// How it reads back to the agent that set it.
    pub fn describe(&self) -> String {
        format!("{}, next {}", self.trigger.describe(), when(self.next_run_at))
    }
}

/// Whole units where they divide evenly, because "every 2 hours" is what was
/// asked for and "every 7200 seconds" is the same thing said badly.
pub fn human_gap(secs: u32) -> String {
    const MINUTE: u32 = 60;
    const HOUR: u32 = 60 * MINUTE;
    const DAY: u32 = 24 * HOUR;

    let (n, unit) = if secs.is_multiple_of(DAY) && secs >= DAY {
        (secs / DAY, "day")
    } else if secs.is_multiple_of(HOUR) && secs >= HOUR {
        (secs / HOUR, "hour")
    } else if secs.is_multiple_of(MINUTE) && secs >= MINUTE {
        (secs / MINUTE, "minute")
    } else {
        (secs, "second")
    };
    if n == 1 {
        unit.to_string()
    } else {
        format!("{n} {unit}s")
    }
}

fn when(at: i64) -> String {
    let seconds = (at - super::now_ms()) / 1000;
    if seconds <= 0 {
        return "now".to_string();
    }
    format!("in {}", human_gap(seconds as u32))
}

/// The shortest gap a routine may repeat on.
///
/// A minute is already fast for something that spends model calls, and anything
/// shorter is a way to spend a budget by accident.
pub const MIN_EVERY_SECS: u32 = 60;

/// A year. Anything longer is a mistake in arithmetic somewhere.
pub const MAX_DELAY_SECS: u32 = 365 * 24 * 60 * 60;

/// Long enough to say what a routine is for at a glance, short enough to fit
/// the row it is drawn in.
pub const MAX_NAME_LEN: usize = 64;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum RoutineError {
    #[error("a routine needs something to do")]
    Empty,
    #[error("a routine's name must be {MAX_NAME_LEN} characters or fewer")]
    NameTooLong,
    #[error("the shortest repeat is {MIN_EVERY_SECS} seconds, got {got}")]
    TooOften { got: u32 },
    #[error("that is further ahead than this can schedule")]
    TooFar,
}

/// Checks what was asked for before it becomes a row.
pub fn validate(
    name: &str,
    what: &str,
    trigger: Trigger,
    in_secs: Option<u32>,
) -> Result<(), RoutineError> {
    if what.trim().is_empty() {
        return Err(RoutineError::Empty);
    }
    if name.trim().chars().count() > MAX_NAME_LEN {
        return Err(RoutineError::NameTooLong);
    }
    if let Trigger::Every(every) = trigger {
        if every < MIN_EVERY_SECS {
            return Err(RoutineError::TooOften { got: every });
        }
        if every > MAX_DELAY_SECS {
            return Err(RoutineError::TooFar);
        }
    }
    if in_secs.is_some_and(|delay| delay > MAX_DELAY_SECS) {
        return Err(RoutineError::TooFar);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveTime;

    /// A local wall-clock moment, as a timestamp. Every calendar assertion in
    /// here is written in local time, because that is the only time a person
    /// setting "weekdays at nine" is thinking in.
    fn at(y: i32, m: u32, d: u32, hour: u32, minute: u32) -> i64 {
        let date = NaiveDate::from_ymd_opt(y, m, d).unwrap();
        let time = NaiveTime::from_hms_opt(hour, minute, 0).unwrap();
        instant(date.and_time(time)).unwrap()
    }

    fn routine(trigger: Trigger, next_run_at: i64) -> Routine {
        Routine {
            id: RoutineId::new(),
            agent_id: AgentId::new(),
            name: String::new(),
            what: "check".into(),
            trigger,
            active: true,
            next_run_at,
            last_run_at: None,
            created_at: 0,
        }
    }

    #[test]
    fn a_gap_is_described_in_the_units_it_was_asked_for() {
        assert_eq!(human_gap(60), "minute");
        assert_eq!(human_gap(300), "5 minutes");
        assert_eq!(human_gap(3600), "hour");
        assert_eq!(human_gap(5 * 3600), "5 hours");
        assert_eq!(human_gap(86400), "day");
        assert_eq!(human_gap(90), "90 seconds", "an uneven gap stays in seconds");
    }

    #[test]
    fn a_trigger_survives_the_round_trip_through_its_stored_form() {
        for trigger in [
            Trigger::Once,
            Trigger::Every(3600),
            Trigger::Every(18_000),
            Trigger::Daily,
            Trigger::Weekdays,
            Trigger::Weekly,
            Trigger::Monthly,
        ] {
            assert_eq!(Trigger::parse(&trigger.as_str()), Some(trigger), "{trigger:?}");
        }
        assert_eq!(Trigger::parse("nonsense"), None);
        assert_eq!(Trigger::parse("every:"), None);
        assert_eq!(Trigger::parse("every:-1"), None);
    }

    #[test]
    fn a_trigger_crosses_the_ipc_boundary_as_the_string_it_is_stored_as() {
        // The webview and SQLite have to be reading the same thing. A derived
        // enum would hand the webview `{"kind":"every","secs":3600}` and the
        // database `every:3600`, and the frontend parses neither by accident.
        let routine = routine(Trigger::Every(3600), 0);
        let json = serde_json::to_value(&routine).unwrap();
        assert_eq!(json["trigger"], serde_json::json!("every:3600"));

        let weekdays = serde_json::to_value(Trigger::Weekdays).unwrap();
        assert_eq!(weekdays, serde_json::json!("weekdays"));
        assert_eq!(
            serde_json::from_value::<Trigger>(serde_json::json!("monthly")).unwrap(),
            Trigger::Monthly
        );
        // And a value this build does not know is refused rather than guessed.
        assert!(serde_json::from_value::<Trigger>(serde_json::json!("fortnightly")).is_err());
    }

    #[test]
    fn a_repeat_is_counted_from_when_it_ran_not_from_when_it_was_due() {
        // A machine asleep through three slots must not wake and fire three
        // times to catch up.
        let routine = routine(Trigger::Every(3600), 1_000);
        assert_eq!(routine.after_running(10_000_000), Some(10_000_000 + 3_600_000));
    }

    #[test]
    fn a_one_shot_has_no_next_time() {
        let routine = routine(Trigger::Once, 1_000);
        assert!(!routine.repeats());
        assert_eq!(routine.after_running(5_000), None);
    }

    #[test]
    fn a_daily_routine_keeps_its_hour_rather_than_adding_a_day_of_seconds() {
        // 2025-03-09 is when the US springs forward. A day counted in seconds
        // would move a 9am routine to 10am and leave it there.
        let slot = at(2025, 3, 8, 9, 0);
        let next = Trigger::Daily.next_after(slot, slot + 1000).unwrap();
        assert_eq!(next, at(2025, 3, 9, 9, 0));
        let after = Trigger::Daily.next_after(next, next + 1000).unwrap();
        assert_eq!(after, at(2025, 3, 10, 9, 0));
    }

    #[test]
    fn weekdays_skip_the_weekend_and_land_on_monday() {
        // 2025-01-03 is a Friday.
        let friday = at(2025, 1, 3, 9, 0);
        let next = Trigger::Weekdays.next_after(friday, friday + 1000).unwrap();
        assert_eq!(next, at(2025, 1, 6, 9, 0), "Friday's next weekday is Monday");

        let monday = at(2025, 1, 6, 9, 0);
        assert_eq!(
            Trigger::Weekdays.next_after(monday, monday + 1000).unwrap(),
            at(2025, 1, 7, 9, 0)
        );
    }

    #[test]
    fn a_weekday_routine_slept_through_a_weekend_fires_once_not_three_times() {
        // Due Friday, machine off until Monday afternoon. The next slot is
        // Tuesday morning: one firing, not one for each missed day.
        let friday = at(2025, 1, 3, 9, 0);
        let monday_afternoon = at(2025, 1, 6, 15, 0);
        assert_eq!(
            Trigger::Weekdays.next_after(friday, monday_afternoon).unwrap(),
            at(2025, 1, 7, 9, 0)
        );
    }

    #[test]
    fn a_weekly_routine_stays_on_its_weekday() {
        let thursday = at(2025, 1, 2, 14, 30);
        let next = Trigger::Weekly.next_after(thursday, thursday + 1000).unwrap();
        assert_eq!(next, at(2025, 1, 9, 14, 30));
    }

    #[test]
    fn a_monthly_routine_on_the_31st_does_not_walk_backwards_down_the_calendar() {
        // Clamping without re-anchoring turns the 31st into the 28th and then
        // keeps it there for the rest of the year.
        let jan = at(2025, 1, 31, 8, 0);
        let feb = Trigger::Monthly.next_after(jan, jan + 1000).unwrap();
        assert_eq!(feb, at(2025, 2, 28, 8, 0), "February has no 31st");
        let mar = Trigger::Monthly.next_after(jan, feb + 1000).unwrap();
        assert_eq!(mar, at(2025, 3, 31, 8, 0), "March does, so it is the 31st again");
    }

    #[test]
    fn a_monthly_routine_crosses_the_year() {
        let dec = at(2025, 12, 15, 10, 0);
        assert_eq!(Trigger::Monthly.next_after(dec, dec + 1000).unwrap(), at(2026, 1, 15, 10, 0));
    }

    #[test]
    fn a_decade_stale_routine_still_finds_its_next_slot() {
        // The search is bounded, and the bound has to clear the worst case a
        // machine left switched off can produce.
        let long_ago = at(2015, 1, 5, 9, 0);
        let now = at(2025, 6, 10, 12, 0);
        let next = Trigger::Daily.next_after(long_ago, now).unwrap();
        assert_eq!(next, at(2025, 6, 11, 9, 0));
    }

    #[test]
    fn a_repeat_with_no_stated_start_waits_a_whole_interval() {
        let now = at(2025, 1, 2, 9, 0);
        assert_eq!(Trigger::Every(3600).first_run(now, None), now + 3_600_000);
        assert_eq!(Trigger::Daily.first_run(now, None), at(2025, 1, 3, 9, 0));
        assert_eq!(Trigger::Once.first_run(now, None), now, "a one-shot with no delay is now");
    }

    #[test]
    fn a_first_run_asked_for_on_a_weekend_moves_to_monday() {
        // The operator picks a time of day, not a day. Honouring a Saturday
        // start on a routine that says it never runs at the weekend would fire
        // it on a day its own label rules out.
        let friday = at(2025, 1, 3, 12, 0);
        let saturday = at(2025, 1, 4, 9, 0);
        let delay = ((saturday - friday) / 1000) as u32;
        assert_eq!(Trigger::Weekdays.first_run(friday, Some(delay)), at(2025, 1, 6, 9, 0));

        // A weekday start is left exactly where it was asked for.
        let monday = at(2025, 1, 6, 9, 0);
        let to_monday = ((monday - friday) / 1000) as u32;
        assert_eq!(Trigger::Weekdays.first_run(friday, Some(to_monday)), monday);
    }

    #[test]
    fn a_routine_without_a_name_is_titled_by_what_it_does() {
        let mut r = routine(Trigger::Daily, 0);
        r.what = "check the listings".into();
        assert_eq!(r.title(), "check the listings");
        r.name = "  ".into();
        assert_eq!(r.title(), "check the listings", "a blank name is not a name");
        r.name = "Listings sweep".into();
        assert_eq!(r.title(), "Listings sweep");
    }

    #[test]
    fn a_routine_that_does_nothing_or_runs_constantly_is_refused() {
        assert_eq!(validate("", "  ", Trigger::Daily, None), Err(RoutineError::Empty));
        assert_eq!(
            validate("", "x", Trigger::Every(5), None),
            Err(RoutineError::TooOften { got: 5 })
        );
        assert_eq!(
            validate("", "x", Trigger::Once, Some(MAX_DELAY_SECS + 1)),
            Err(RoutineError::TooFar)
        );
        assert_eq!(
            validate(&"n".repeat(MAX_NAME_LEN + 1), "x", Trigger::Daily, None),
            Err(RoutineError::NameTooLong)
        );
        assert_eq!(validate("Sweep", "x", Trigger::Every(3600), Some(60)), Ok(()));
    }
}
