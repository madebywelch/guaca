//! Routines: an agent's own schedule, and whatever else sets one off.
//!
//! An agent sets these for itself. "Check the listings every five hours" and
//! "wake me in an hour" are the same thing with and without a repeat, so both
//! are one row: what to do, and what makes it happen.
//!
//! What is stored is the next due time rather than a running timer, so a
//! schedule survives a restart. Nothing has to be held in memory for a routine
//! to fire tomorrow.
//!
//! Not every routine waits on a clock. A [`Trigger`] is either a [`Cadence`],
//! which owns a moment and is what the scheduler sweeps for, or an
//! [`EventTrigger`], which owns nothing and waits to be told. The second kind
//! has no next moment, which is why the moment is an `Option` here and nullable
//! in SQLite: a routine waiting on Stripe with a slot in it would either fire
//! on the clock or need a sentinel, and a sentinel is a date the operator would
//! eventually be shown.

use chrono::{
    DateTime, Datelike, Duration, Local, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Weekday,
};
use serde::{Deserialize, Serialize};

use super::ids::{AgentId, RoutineId, RunId};

/// What makes a routine fire.
///
/// One string, on the wire and in the database both, which is why this has a
/// hand-written `Serialize`. A derived one would give the webview a tagged
/// object and SQLite a string for the same fact, and the two would drift the
/// first time either gained a variant.
///
/// The string is also what makes a new kind of trigger a new value rather than
/// a new column: `every:3600` and `event:stripe/invoice.payment_failed` are
/// both one `fires` column and one `trigger` field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trigger {
    /// A moment, or a repeat of them. The scheduler owns these.
    Clock(Cadence),
    /// Something happening in a service the group is connected to. No moment:
    /// it waits.
    Event(EventTrigger),
}

/// A repeat on the clock.
///
/// `Daily`, `Weekly` and `Monthly` are deliberately not gaps in seconds even
/// though two of them nearly are. A day is 23 or 25 hours twice a year, and a
/// month is four different lengths, so a routine stored as a gap wanders off
/// the hour it was set for. What is stored is the shape of the repeat; the hour
/// it happens at comes from the slot it is holding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cadence {
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

/// Something happening somewhere else.
///
/// `service` names a connector the group holds and `topic` is that service's
/// own word for what happened, verbatim: `stripe` and
/// `invoice.payment_failed`. Both are identifiers rather than prose, which is
/// what [`EventTrigger::parse`] enforces, and the service is lowered so the
/// stored form is canonical: one routine per event, not one per spelling.
///
/// **Nothing delivers one of these yet.** There is no event source, so a
/// routine triggered this way fires only when the operator presses Test run.
/// What exists is the shape: it stores, it reads back, it is described, it
/// keeps a history, and the scheduler leaves it alone. What is missing is the
/// half that cannot be written without a service to receive from: a webhook or
/// a poll that turns an arriving event into `Runtime::send_from_routine`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventTrigger {
    pub service: String,
    pub topic: String,
}

/// A service name longer than this is prose. Matches `connector::MAX_SERVICE_LEN`,
/// because this names one of those.
pub const MAX_EVENT_SERVICE_LEN: usize = 48;
/// Vendors' event names are long: `customer.subscription.pending_update_expired`.
pub const MAX_EVENT_TOPIC_LEN: usize = 120;

impl EventTrigger {
    /// What marks an event out from a cadence in the stored form.
    pub const PREFIX: &'static str = "event:";

    /// Reads `stripe/invoice.payment_failed`, the part after the prefix.
    ///
    /// Split at the first slash only: a service's own topic names can contain
    /// more of them, and the service is the half this has to be sure of.
    pub fn parse(rest: &str) -> Option<Self> {
        let (service, topic) = rest.trim().split_once('/')?;
        let service = service.trim().to_lowercase();
        let topic = topic.trim().to_string();
        let sound = |part: &str, max: usize| {
            !part.is_empty() && part.chars().count() <= max && !part.contains(char::is_whitespace)
        };
        if !sound(&service, MAX_EVENT_SERVICE_LEN) || !sound(&topic, MAX_EVENT_TOPIC_LEN) {
            return None;
        }
        Some(EventTrigger { service, topic })
    }

    pub fn as_str(&self) -> String {
        format!("{}{}/{}", Self::PREFIX, self.service, self.topic)
    }

    /// How it reads to whoever set it: `when Stripe reports invoice.paid`.
    pub fn describe(&self) -> String {
        format!("when {} reports {}", titled(&self.service), self.topic)
    }
}

/// `stripe` as `Stripe`. The service is stored lowered so it can be matched;
/// it is shown the way the operator wrote it down.
fn titled(service: &str) -> String {
    let mut chars = service.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

impl Cadence {
    /// The stored form. Parsed back by [`Cadence::parse`].
    pub fn as_str(&self) -> String {
        match self {
            Cadence::Once => "once".to_string(),
            Cadence::Every(secs) => format!("every:{secs}"),
            Cadence::Daily => "daily".to_string(),
            Cadence::Weekdays => "weekdays".to_string(),
            Cadence::Weekly => "weekly".to_string(),
            Cadence::Monthly => "monthly".to_string(),
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "once" => Some(Cadence::Once),
            "daily" => Some(Cadence::Daily),
            "weekdays" => Some(Cadence::Weekdays),
            "weekly" => Some(Cadence::Weekly),
            "monthly" => Some(Cadence::Monthly),
            other => other.strip_prefix("every:")?.parse().ok().map(Cadence::Every),
        }
    }

    pub fn repeats(&self) -> bool {
        !matches!(self, Cadence::Once)
    }

    /// How it reads to whoever set it.
    pub fn describe(&self) -> String {
        match self {
            Cadence::Once => "once".to_string(),
            Cadence::Every(secs) => format!("every {}", human_gap(*secs)),
            Cadence::Daily => "every day".to_string(),
            Cadence::Weekdays => "every weekday".to_string(),
            Cadence::Weekly => "every week".to_string(),
            Cadence::Monthly => "every month".to_string(),
        }
    }

    /// Whether a moment is one this cadence would ever fire on.
    ///
    /// Only weekdays can say no. It exists so a first run the operator asked
    /// for on a Saturday moves to Monday instead of firing on a day the
    /// routine says it never runs.
    pub fn accepts(&self, at: i64) -> bool {
        match self {
            Cadence::Weekdays => match local(at) {
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
            Cadence::Once => None,
            // A gap is counted from when it ran rather than from when it was
            // due, for the same no-catch-up reason. There is no hour to hold
            // on to here, so there is nothing to anchor to.
            Cadence::Every(secs) => Some(now + i64::from(*secs) * 1000),
            _ => self.next_calendar_slot(slot, now),
        }
    }

    /// When a routine just set should first fire.
    ///
    /// A repeat with no stated start waits one whole interval, which is what
    /// "every weekday" means to the person who said it: not now and then every
    /// weekday. A stated start is honoured, except on a day this cadence never
    /// fires on.
    pub fn first_run(&self, now: i64, in_secs: Option<u32>) -> i64 {
        let asked = in_secs.map(|delay| now + i64::from(delay) * 1000);
        match (self, asked) {
            (Cadence::Once, _) => asked.unwrap_or(now),
            (_, Some(at)) if self.accepts(at) => at,
            (Cadence::Every(secs), None) => now + i64::from(*secs) * 1000,
            // A start on a day this never fires on, or none at all: the anchor
            // keeps the hour and the cadence picks the day.
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

    /// The `n`th date this cadence fires on after `anchor`.
    ///
    /// Counted from the anchor every time rather than from the previous answer,
    /// so a monthly routine set on the 31st is the 28th in February and the
    /// 31st again in March instead of walking backward down the calendar.
    fn nth_date(&self, anchor: NaiveDate, n: i64) -> Option<NaiveDate> {
        match self {
            Cadence::Daily => anchor.checked_add_signed(Duration::days(n)),
            Cadence::Weekly => anchor.checked_add_signed(Duration::days(n * 7)),
            Cadence::Weekdays => nth_weekday_after(anchor, n),
            Cadence::Monthly => months_after(anchor, n),
            Cadence::Once | Cadence::Every(_) => None,
        }
    }
}

impl Trigger {
    /// The stored form. Parsed back by [`Trigger::parse`].
    pub fn as_str(&self) -> String {
        match self {
            Trigger::Clock(cadence) => cadence.as_str(),
            Trigger::Event(event) => event.as_str(),
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        match value.strip_prefix(EventTrigger::PREFIX) {
            Some(rest) => EventTrigger::parse(rest).map(Trigger::Event),
            None => Cadence::parse(value).map(Trigger::Clock),
        }
    }

    /// The cadence, when this is one. `None` is a trigger the scheduler has no
    /// business looking at.
    pub fn cadence(&self) -> Option<Cadence> {
        match self {
            Trigger::Clock(cadence) => Some(*cadence),
            Trigger::Event(_) => None,
        }
    }

    /// Whether this fires more than once.
    ///
    /// An event trigger does: it fires every time the thing it names happens,
    /// which is exactly why it must not be deleted after one firing the way a
    /// one-shot is.
    pub fn repeats(&self) -> bool {
        match self {
            Trigger::Clock(cadence) => cadence.repeats(),
            Trigger::Event(_) => true,
        }
    }

    /// How it reads to whoever set it.
    pub fn describe(&self) -> String {
        match self {
            Trigger::Clock(cadence) => cadence.describe(),
            Trigger::Event(event) => event.describe(),
        }
    }

    /// When a routine just set should first fire, or `None` when it does not
    /// wait on the clock at all.
    pub fn first_run(&self, now: i64, in_secs: Option<u32>) -> Option<i64> {
        self.cadence().map(|cadence| cadence.first_run(now, in_secs))
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
    /// Whether a firing that comes due while the agent is already working is
    /// dropped instead of queued behind whatever it is doing.
    ///
    /// For the sweep that must not stack: an agent still working through the
    /// last hour's listings does not want this hour's waiting for it, and the
    /// next slot is a few minutes away regardless.
    ///
    /// Only ever true on a routine that repeats, which [`validate`] is what
    /// enforces. A one-off dropped is a one-off that never happens, and the
    /// slot it was holding goes with it.
    pub skip_if_working: bool,
    /// The moment it is next due, for a routine that waits on the clock.
    ///
    /// `None` is a routine that does not: an event trigger fires when its event
    /// arrives and holds no slot in the meantime. The scheduler asks for slots
    /// at or before now, so an empty one is never due, and that is the whole
    /// mechanism keeping event triggers out of the sweep.
    pub next_run_at: Option<i64>,
    pub last_run_at: Option<i64>,
    pub created_at: i64,
}

/// What becomes of a routine's slot once it has fired.
///
/// Three answers rather than an `Option<i64>`, because "nothing on the clock"
/// and "finished" mean opposite things to the row: one keeps it and one deletes
/// it, and reading them off the same `None` deleted every event routine the
/// first time it fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextSlot {
    /// Due again at this moment.
    Due(i64),
    /// Holding no slot, and still standing.
    Waiting,
    /// Done. The row goes.
    Done,
}

/// What happened at one firing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunKind {
    /// It came due and was delivered.
    Scheduled,
    /// The operator pressed the button. Nothing about the schedule moved.
    Test,
    /// It came due, the agent was already working, and the routine says not to
    /// land on that. Nothing was delivered and the slot moved on anyway.
    ///
    /// Recorded rather than passed over in silence: a firing that does not
    /// happen leaves a gap in this history, and a gap is what a broken
    /// scheduler looks like too.
    Skipped,
}

impl RunKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RunKind::Scheduled => "scheduled",
            RunKind::Test => "test",
            RunKind::Skipped => "skipped",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "scheduled" => Some(RunKind::Scheduled),
            "test" => Some(RunKind::Test),
            "skipped" => Some(RunKind::Skipped),
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
/// `spent` is the second of those, read back at the same time, because "did
/// Tuesday's sweep actually do anything" is answered by whether the firing
/// bought any model calls and not by the fact that it was delivered.
///
/// A skipped firing has no run at all, which is why the id is optional. An
/// invented one would read back exactly like a delivery that spent nothing,
/// and telling those two apart is the whole job of this row.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineRun {
    pub run_id: Option<RunId>,
    pub kind: RunKind,
    pub at: i64,
    pub spent: super::usage::Tokens,
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

    /// The same, cut down to something that fits one line of a list.
    ///
    /// An agent naming its own routine is optional, so the title is often the
    /// instruction, and an instruction is written to be acted on with no other
    /// context: several sentences. Whoever is reading a list is recognizing a
    /// routine rather than reading it, and the whole instruction is what the
    /// `list` action is for. Cut to the width a name is already held to, so a
    /// named row and an unnamed one are the same shape.
    pub fn short_title(&self) -> String {
        let title = self.title().trim();
        if title.chars().count() <= MAX_NAME_LEN {
            return title.to_string();
        }
        let cut: String = title.chars().take(MAX_NAME_LEN).collect();
        match cut.rfind(char::is_whitespace) {
            Some(space) => format!("{}…", cut[..space].trim_end()),
            None => format!("{cut}…"),
        }
    }

    /// What happens to its slot, given that it just ran at `now`.
    pub fn after_running(&self, now: i64) -> NextSlot {
        match self.trigger.cadence() {
            // A clock routine always holds a slot. An empty one would be a row
            // written by something that did not know that, and `now` is the
            // reading that keeps the cadence's own hour closest to true.
            Some(cadence) => match cadence.next_after(self.next_run_at.unwrap_or(now), now) {
                Some(next) => NextSlot::Due(next),
                None => NextSlot::Done,
            },
            None => NextSlot::Waiting,
        }
    }

    /// How it reads back to the agent that set it.
    ///
    /// A routine the operator has switched off says so instead of claiming a
    /// next firing. It still holds the slot it was holding, so the countdown
    /// is there to be printed, and an agent told "every weekday, next in 15
    /// hours" about a row that will not fire reports work as being in hand
    /// that nobody is going to do.
    pub fn describe(&self) -> String {
        let mut out = if !self.active {
            format!("{}, switched off by the operator", self.trigger.describe())
        } else {
            match self.next_run_at {
                Some(at) => format!("{}, next {}", self.trigger.describe(), when(at)),
                // Nothing to promise: it happens when the event does.
                None => self.trigger.describe(),
            }
        };
        // Said here rather than only in the panel, because this is the line an
        // agent reads its own schedule off. One that cannot see which of its
        // routines drops a firing cannot answer why yesterday's did not run,
        // and books a second one to cover the gap.
        if self.skip_if_working {
            out.push_str(", dropped if you are already working");
        }
        out
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
    format!("in {}", rough_gap(seconds as u32))
}

/// A gap to a moment, as the nearest whole unit of something.
///
/// [`human_gap`] is exact because a repeat is a number somebody chose: "every 2
/// hours" has to read back as that and not as an approximation of it. The gap
/// to a next firing is not a number anybody chose, and it is almost never a
/// round multiple of anything, so exactness there produced "next in 51823
/// seconds": the true answer to a question nobody asked, in every prompt and
/// every reply that mentions a schedule.
fn rough_gap(secs: u32) -> String {
    const MINUTE: u32 = 60;
    const HOUR: u32 = 60 * MINUTE;
    const DAY: u32 = 24 * HOUR;

    if secs < MINUTE {
        return "under a minute".to_string();
    }
    // Which unit to use is decided on the rounded number rather than on the raw
    // seconds, or a slot 59 and a half minutes out reads as 60 minutes. The
    // boundaries sit where the larger unit starts being the easier read: 30
    // hours is clearer than a day and a bit.
    let minutes = (secs + MINUTE / 2) / MINUTE;
    let hours = (secs + HOUR / 2) / HOUR;
    let (n, unit) = if minutes < 60 {
        (minutes, "minute")
    } else if hours < 48 {
        (hours, "hour")
    } else {
        ((secs + DAY / 2) / DAY, "day")
    };
    // "in minute" is what a bare unit reads as here, unlike in "every minute",
    // which is the wording `human_gap` is for.
    match (n, unit) {
        (1, "hour") => "an hour".to_string(),
        (1, unit) => format!("a {unit}"),
        (n, unit) => format!("{n} {unit}s"),
    }
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
    #[error("an event trigger has no start time: it fires when {service} reports {topic}")]
    EventHasNoStart { service: String, topic: String },
    #[error(
        "only a routine that repeats can skip a firing: a one-off skipped is one that never \
         happens at all"
    )]
    SkipNeedsARepeat,
}

/// Checks what was asked for before it becomes a row.
pub fn validate(
    name: &str,
    what: &str,
    trigger: &Trigger,
    in_secs: Option<u32>,
    skip_if_working: bool,
) -> Result<(), RoutineError> {
    if what.trim().is_empty() {
        return Err(RoutineError::Empty);
    }
    if name.trim().chars().count() > MAX_NAME_LEN {
        return Err(RoutineError::NameTooLong);
    }
    match trigger {
        Trigger::Clock(Cadence::Every(every)) => {
            if *every < MIN_EVERY_SECS {
                return Err(RoutineError::TooOften { got: *every });
            }
            if *every > MAX_DELAY_SECS {
                return Err(RoutineError::TooFar);
            }
        }
        // A delay is a statement about a clock, so asking for one here is a
        // misunderstanding worth naming rather than a field to drop silently:
        // whoever sent it thinks they have scheduled something.
        Trigger::Event(event) if in_secs.is_some() => {
            return Err(RoutineError::EventHasNoStart {
                service: titled(&event.service),
                topic: event.topic.clone(),
            })
        }
        _ => {}
    }
    if in_secs.is_some_and(|delay| delay > MAX_DELAY_SECS) {
        return Err(RoutineError::TooFar);
    }
    // Refused rather than stored and honored, because honoring it destroys the
    // routine: skipping moves the slot on, and the slot a one-off holds is the
    // only one it has, so the row would be deleted having done nothing. The
    // operator would find an empty list where their alarm used to be.
    if skip_if_working && !trigger.repeats() {
        return Err(RoutineError::SkipNeedsARepeat);
    }
    Ok(())
}

/// Where an edited routine's next firing lands.
///
/// Three cases, and the difference between them is what was asked for. A stated
/// time is honoured. An untouched time keeps the slot it was holding, because
/// correcting a typo must not push the schedule to tomorrow. And a trigger
/// swapped for one that would never fire at that moment has to move: "every
/// hour" turned into "every weekday" keeps its hour but cannot keep its
/// Saturday, or the label and the firing disagree from the moment it is saved.
///
/// A trigger that is not a clock lands nowhere at all, whatever was asked for.
///
/// One rule for both editors. An operator editing in the panel and an agent
/// calling `schedule` with `update` are changing the same row, and two answers
/// to "when is it next due" would be two schedules.
pub fn next_slot_for(trigger: &Trigger, existing: &Routine, in_secs: Option<u32>) -> Option<i64> {
    let cadence = trigger.cadence()?;
    let now = super::now_ms();
    match (in_secs, existing.next_run_at) {
        (Some(_), _) => Some(cadence.first_run(now, in_secs)),
        // Coming back to the clock from a trigger that held no slot: there is
        // nothing to keep, so it starts one interval out like a new routine.
        (None, None) => Some(cadence.first_run(now, None)),
        (None, Some(slot)) if cadence.accepts(slot) => Some(slot),
        (None, Some(slot)) => Some(cadence.next_after(slot, now).unwrap_or(slot)),
    }
}

/// Words that carry the subject of an instruction, for [`same_job`].
///
/// Anything shorter than four characters is grammar rather than subject
/// matter: left in, "check the listings" and "email the operator" score on
/// `the` and every pair of instructions looks related.
fn subject_words(text: &str) -> std::collections::HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.chars().count() >= 4)
        .map(str::to_string)
        .collect()
}

/// Whether two instructions read like the same job.
///
/// Deliberately loose, and deliberately only ever used to say something out
/// loud. An agent that has just written a second routine for work it already
/// had standing is told so, with both ids, while it still knows which one it
/// meant; a false positive costs a sentence it can ignore.
///
/// It is not a refusal, and it must not become one. Nothing here can tell
/// "move the sweep to ten" from "sweep at ten as well": both arrive as the
/// same instruction on a different clock, so a guard that refused the second
/// would refuse honest work, and the agent's way around it would be to reword
/// the instruction until it got through.
pub fn same_job(a: &str, b: &str) -> bool {
    let (a, b) = (subject_words(a), subject_words(b));
    let fewer = a.len().min(b.len());
    if fewer == 0 {
        // Two instructions with no subject words between them. Nothing to
        // compare, so nothing is claimed.
        return false;
    }
    // Three subject words in five. Below that, two routines that both mention
    // the listings are usually two jobs; at or above it they are usually one.
    a.intersection(&b).count() * 5 >= fewer * 3
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::usage::Tokens;
    use chrono::NaiveTime;

    /// A local wall-clock moment, as a timestamp. Every calendar assertion in
    /// here is written in local time, because that is the only time a person
    /// setting "weekdays at nine" is thinking in.
    fn at(y: i32, m: u32, d: u32, hour: u32, minute: u32) -> i64 {
        let date = NaiveDate::from_ymd_opt(y, m, d).unwrap();
        let time = NaiveTime::from_hms_opt(hour, minute, 0).unwrap();
        instant(date.and_time(time)).unwrap()
    }

    fn clock(cadence: Cadence) -> Trigger {
        Trigger::Clock(cadence)
    }

    fn event(service: &str, topic: &str) -> Trigger {
        Trigger::Event(EventTrigger { service: service.into(), topic: topic.into() })
    }

    fn routine(trigger: Trigger, next_run_at: Option<i64>) -> Routine {
        Routine {
            id: RoutineId::new(),
            agent_id: AgentId::new(),
            name: String::new(),
            what: "check".into(),
            trigger,
            active: true,
            skip_if_working: false,
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
            clock(Cadence::Once),
            clock(Cadence::Every(3600)),
            clock(Cadence::Every(18_000)),
            clock(Cadence::Daily),
            clock(Cadence::Weekdays),
            clock(Cadence::Weekly),
            clock(Cadence::Monthly),
            event("stripe", "invoice.payment_failed"),
            event("linear", "issue.assigned"),
        ] {
            assert_eq!(Trigger::parse(&trigger.as_str()), Some(trigger.clone()), "{trigger:?}");
        }
        assert_eq!(Trigger::parse("nonsense"), None);
        assert_eq!(Trigger::parse("every:"), None);
        assert_eq!(Trigger::parse("every:-1"), None);
    }

    #[test]
    fn an_event_is_two_identifiers_and_is_refused_when_it_is_not() {
        // The stored form is a key: a future event source will look a routine
        // up by it. Prose in either half is a routine nothing will ever match.
        assert_eq!(
            Trigger::parse("event:Stripe/invoice.paid"),
            Some(event("stripe", "invoice.paid")),
            "the service is lowered, so one event is not two routines"
        );
        // The topic is the vendor's identifier and is kept exactly.
        assert_eq!(
            Trigger::parse("event:github/Issues.Opened"),
            Some(event("github", "Issues.Opened"))
        );
        // A topic of its own may carry slashes; the service may not.
        assert_eq!(
            Trigger::parse("event:hubspot/deal/stage.changed"),
            Some(event("hubspot", "deal/stage.changed"))
        );

        assert_eq!(Trigger::parse("event:stripe"), None, "an event needs a topic");
        assert_eq!(Trigger::parse("event:/invoice.paid"), None, "and a service");
        assert_eq!(Trigger::parse("event:stripe/"), None);
        assert_eq!(Trigger::parse("event:stripe/an invoice failed"), None, "not a sentence");
        assert_eq!(Trigger::parse(&format!("event:{}/x", "s".repeat(49))), None, "not a paragraph");
    }

    #[test]
    fn a_trigger_crosses_the_ipc_boundary_as_the_string_it_is_stored_as() {
        // The webview and SQLite have to be reading the same thing. A derived
        // enum would hand the webview `{"kind":"every","secs":3600}` and the
        // database `every:3600`, and the frontend parses neither by accident.
        let hourly = routine(clock(Cadence::Every(3600)), Some(0));
        let json = serde_json::to_value(&hourly).unwrap();
        assert_eq!(json["trigger"], serde_json::json!("every:3600"));

        let weekdays = serde_json::to_value(clock(Cadence::Weekdays)).unwrap();
        assert_eq!(weekdays, serde_json::json!("weekdays"));
        assert_eq!(
            serde_json::from_value::<Trigger>(serde_json::json!("monthly")).unwrap(),
            clock(Cadence::Monthly)
        );

        // An event trigger is one string too, and the routine holding one says
        // plainly that it has no next firing rather than inventing a date.
        let waiting = routine(event("stripe", "invoice.payment_failed"), None);
        let json = serde_json::to_value(&waiting).unwrap();
        assert_eq!(json["trigger"], serde_json::json!("event:stripe/invoice.payment_failed"));
        assert_eq!(json["nextRunAt"], serde_json::Value::Null);

        // And a value this build does not know is refused rather than guessed.
        assert!(serde_json::from_value::<Trigger>(serde_json::json!("fortnightly")).is_err());
        assert!(serde_json::from_value::<Trigger>(serde_json::json!("event:stripe")).is_err());
    }

    #[test]
    fn an_event_routine_holds_no_slot_and_is_not_finished_by_firing() {
        // Both halves matter. Nothing on the clock keeps it out of the
        // scheduler's sweep; not being finished keeps it from being deleted
        // like a one-shot the first time it fires.
        let trigger = event("stripe", "invoice.payment_failed");
        assert_eq!(trigger.cadence(), None);
        assert_eq!(trigger.first_run(1_000, None), None);
        assert_eq!(trigger.first_run(1_000, Some(60)), None, "a delay does not give it a slot");
        assert!(trigger.repeats(), "it fires every time the event happens");

        let waiting = routine(trigger, None);
        assert_eq!(waiting.after_running(5_000), NextSlot::Waiting);
        assert_eq!(
            waiting.describe(),
            "when Stripe reports invoice.payment_failed",
            "and it promises no next firing, because it does not have one"
        );
    }

    #[test]
    fn a_repeat_is_counted_from_when_it_ran_not_from_when_it_was_due() {
        // A machine asleep through three slots must not wake and fire three
        // times to catch up.
        let routine = routine(clock(Cadence::Every(3600)), Some(1_000));
        assert_eq!(routine.after_running(10_000_000), NextSlot::Due(10_000_000 + 3_600_000));
    }

    #[test]
    fn a_one_shot_has_no_next_time() {
        let routine = routine(clock(Cadence::Once), Some(1_000));
        assert!(!routine.repeats());
        assert_eq!(routine.after_running(5_000), NextSlot::Done);
    }

    #[test]
    fn a_daily_routine_keeps_its_hour_rather_than_adding_a_day_of_seconds() {
        // 2025-03-09 is when the US springs forward. A day counted in seconds
        // would move a 9am routine to 10am and leave it there.
        let slot = at(2025, 3, 8, 9, 0);
        let next = Cadence::Daily.next_after(slot, slot + 1000).unwrap();
        assert_eq!(next, at(2025, 3, 9, 9, 0));
        let after = Cadence::Daily.next_after(next, next + 1000).unwrap();
        assert_eq!(after, at(2025, 3, 10, 9, 0));
    }

    #[test]
    fn weekdays_skip_the_weekend_and_land_on_monday() {
        // 2025-01-03 is a Friday.
        let friday = at(2025, 1, 3, 9, 0);
        let next = Cadence::Weekdays.next_after(friday, friday + 1000).unwrap();
        assert_eq!(next, at(2025, 1, 6, 9, 0), "Friday's next weekday is Monday");

        let monday = at(2025, 1, 6, 9, 0);
        assert_eq!(
            Cadence::Weekdays.next_after(monday, monday + 1000).unwrap(),
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
            Cadence::Weekdays.next_after(friday, monday_afternoon).unwrap(),
            at(2025, 1, 7, 9, 0)
        );
    }

    #[test]
    fn a_weekly_routine_stays_on_its_weekday() {
        let thursday = at(2025, 1, 2, 14, 30);
        let next = Cadence::Weekly.next_after(thursday, thursday + 1000).unwrap();
        assert_eq!(next, at(2025, 1, 9, 14, 30));
    }

    #[test]
    fn a_monthly_routine_on_the_31st_does_not_walk_backwards_down_the_calendar() {
        // Clamping without re-anchoring turns the 31st into the 28th and then
        // keeps it there for the rest of the year.
        let jan = at(2025, 1, 31, 8, 0);
        let feb = Cadence::Monthly.next_after(jan, jan + 1000).unwrap();
        assert_eq!(feb, at(2025, 2, 28, 8, 0), "February has no 31st");
        let mar = Cadence::Monthly.next_after(jan, feb + 1000).unwrap();
        assert_eq!(mar, at(2025, 3, 31, 8, 0), "March does, so it is the 31st again");
    }

    #[test]
    fn a_monthly_routine_crosses_the_year() {
        let dec = at(2025, 12, 15, 10, 0);
        assert_eq!(Cadence::Monthly.next_after(dec, dec + 1000).unwrap(), at(2026, 1, 15, 10, 0));
    }

    #[test]
    fn a_decade_stale_routine_still_finds_its_next_slot() {
        // The search is bounded, and the bound has to clear the worst case a
        // machine left switched off can produce.
        let long_ago = at(2015, 1, 5, 9, 0);
        let now = at(2025, 6, 10, 12, 0);
        let next = Cadence::Daily.next_after(long_ago, now).unwrap();
        assert_eq!(next, at(2025, 6, 11, 9, 0));
    }

    #[test]
    fn a_repeat_with_no_stated_start_waits_a_whole_interval() {
        let now = at(2025, 1, 2, 9, 0);
        assert_eq!(Cadence::Every(3600).first_run(now, None), now + 3_600_000);
        assert_eq!(Cadence::Daily.first_run(now, None), at(2025, 1, 3, 9, 0));
        assert_eq!(Cadence::Once.first_run(now, None), now, "a one-shot with no delay is now");
    }

    #[test]
    fn a_first_run_asked_for_on_a_weekend_moves_to_monday() {
        // The operator picks a time of day, not a day. Honouring a Saturday
        // start on a routine that says it never runs at the weekend would fire
        // it on a day its own label rules out.
        let friday = at(2025, 1, 3, 12, 0);
        let saturday = at(2025, 1, 4, 9, 0);
        let delay = ((saturday - friday) / 1000) as u32;
        assert_eq!(Cadence::Weekdays.first_run(friday, Some(delay)), at(2025, 1, 6, 9, 0));

        // A weekday start is left exactly where it was asked for.
        let monday = at(2025, 1, 6, 9, 0);
        let to_monday = ((monday - friday) / 1000) as u32;
        assert_eq!(Cadence::Weekdays.first_run(friday, Some(to_monday)), monday);
    }

    #[test]
    fn a_stated_start_picks_the_weekday_a_weekly_routine_keeps() {
        // The operator says "every week, Thursday at 9" by asking for the first
        // firing on a Thursday at 9. Nothing else in the row records the day,
        // so a start that was quietly moved would change the cadence itself.
        let monday = at(2025, 6, 9, 12, 0);
        let thursday = at(2025, 6, 12, 9, 0);
        let delay = ((thursday - monday) / 1000) as u32;
        assert_eq!(Cadence::Weekly.first_run(monday, Some(delay)), thursday);
        assert_eq!(
            Cadence::Weekly.next_after(thursday, thursday + 1000).unwrap(),
            at(2025, 6, 19, 9, 0),
            "and it stays on that Thursday"
        );
    }

    #[test]
    fn a_routine_without_a_name_is_titled_by_what_it_does() {
        let mut r = routine(clock(Cadence::Daily), Some(0));
        r.what = "check the listings".into();
        assert_eq!(r.title(), "check the listings");
        r.name = "  ".into();
        assert_eq!(r.title(), "check the listings", "a blank name is not a name");
        r.name = "Listings sweep".into();
        assert_eq!(r.title(), "Listings sweep");
    }

    #[test]
    fn a_routine_that_does_nothing_or_runs_constantly_is_refused() {
        assert_eq!(
            validate("", "  ", &clock(Cadence::Daily), None, false),
            Err(RoutineError::Empty)
        );
        assert_eq!(
            validate("", "x", &clock(Cadence::Every(5)), None, false),
            Err(RoutineError::TooOften { got: 5 })
        );
        assert_eq!(
            validate("", "x", &clock(Cadence::Once), Some(MAX_DELAY_SECS + 1), false),
            Err(RoutineError::TooFar)
        );
        assert_eq!(
            validate(&"n".repeat(MAX_NAME_LEN + 1), "x", &clock(Cadence::Daily), None, false),
            Err(RoutineError::NameTooLong)
        );
        assert_eq!(validate("Sweep", "x", &clock(Cadence::Every(3600)), Some(60), false), Ok(()));
    }

    #[test]
    fn a_start_time_on_an_event_trigger_is_refused_rather_than_dropped() {
        // Silently ignoring it would leave whoever sent it believing they had
        // scheduled something, and the refusal has to say what will happen
        // instead: an error an agent reads mid-turn needs a way forward.
        let trigger = event("stripe", "invoice.payment_failed");
        let refused = validate("Dunning", "chase it", &trigger, Some(3600), false).unwrap_err();
        assert_eq!(
            refused.to_string(),
            "an event trigger has no start time: it fires when Stripe reports \
             invoice.payment_failed"
        );
        assert_eq!(validate("Dunning", "chase it", &trigger, None, false), Ok(()));
    }

    #[test]
    fn only_something_that_repeats_can_skip_a_firing() {
        // A skip moves the slot on, and the slot a one-off holds is the only
        // one it has: honoring the pair would delete the row having done
        // nothing, and the operator would find an empty list where their alarm
        // was. Refused where it is asked for rather than dropped quietly.
        let refused =
            validate("Wake me", "check the listings", &clock(Cadence::Once), Some(3600), true)
                .unwrap_err();
        assert_eq!(refused, RoutineError::SkipNeedsARepeat);
        assert!(
            refused.to_string().contains("never happens"),
            "an error read mid-turn has to say what would go wrong: {refused}"
        );

        // Every repeat takes it, including one waiting on an event: that fires
        // each time the event arrives, so there is always a next one.
        for trigger in [
            clock(Cadence::Daily),
            clock(Cadence::Every(3600)),
            event("stripe", "invoice.payment_failed"),
        ] {
            assert_eq!(validate("Sweep", "check", &trigger, None, true), Ok(()), "{trigger:?}");
        }

        // And a one-off is fine as long as nobody asked for the skip.
        assert_eq!(validate("Wake me", "check", &clock(Cadence::Once), Some(3600), false), Ok(()));
    }

    #[test]
    fn a_routine_that_drops_a_firing_says_so_to_the_agent_keeping_it() {
        // This line is the whole of what an agent reads its own schedule off,
        // in the prompt and from `list`. One that cannot see which of its
        // routines skips cannot say why yesterday's did not run, and books a
        // second one to cover the gap.
        let mut r = routine(clock(Cadence::Daily), Some(at(2025, 6, 10, 9, 0)));
        assert!(!r.describe().contains("dropped"), "silent unless it was asked for");

        r.skip_if_working = true;
        assert!(
            r.describe().starts_with("every day, next"),
            "the cadence and the countdown still lead: {}",
            r.describe()
        );
        assert!(r.describe().ends_with(", dropped if you are already working"));

        // Both facts survive together. A routine switched off that also skips
        // must not report either one instead of the other.
        r.active = false;
        assert_eq!(
            r.describe(),
            "every day, switched off by the operator, dropped if you are already working"
        );
    }

    #[test]
    fn a_firing_carries_what_it_spent() {
        // The history exists to answer "has this been working", and a delivery
        // that bought no model call is a routine that did not run. Nothing else
        // in the row distinguishes the two.
        let run = RoutineRun {
            run_id: Some(RunId::new()),
            kind: RunKind::Scheduled,
            at: 1_000,
            spent: Tokens { prompt: 900, completion: 100, cost: Some(0.002), calls: 2 },
        };
        let json = serde_json::to_value(&run).unwrap();
        assert_eq!(json["kind"], serde_json::json!("scheduled"));
        assert_eq!(json["spent"]["calls"], serde_json::json!(2));
        assert_eq!(json["spent"]["cost"], serde_json::json!(0.002));
    }

    #[test]
    fn a_next_firing_is_described_in_a_unit_a_reader_can_use() {
        // A slot is a moment on the clock, not a gap somebody chose, so it is
        // almost never a round number of anything. Printed exactly, an agent
        // was told "next in 51823 seconds" on every turn.
        let now = crate::domain::now_ms();
        assert_eq!(when(now + 51_823_000), "in 14 hours");
        assert_eq!(when(now + 3_599_000), "in an hour");
        assert_eq!(when(now + 86_400_000 * 3), "in 3 days");
        assert_eq!(when(now + 30_000), "in under a minute");
        assert_eq!(when(now - 1_000), "now");
        // And a single unit reads as one. "next in minute" was the shape a
        // bare unit takes here, which is not the shape "every minute" takes.
        assert_eq!(when(now + 70_000), "in a minute");
        assert_eq!(when(now + 5_500_000), "in 2 hours");
        // A repeat is still exact: it reads back as the number that was set.
        assert_eq!(human_gap(7200), "2 hours");
    }

    #[test]
    fn correcting_a_routine_leaves_the_slot_it_was_holding_alone() {
        // The edit an operator makes most: a typo in the instruction. Moving
        // the next firing to an interval from now would quietly cancel this
        // morning's run.
        let slot = at(2025, 6, 10, 9, 0);
        let existing = routine(clock(Cadence::Daily), Some(slot));
        assert_eq!(next_slot_for(&clock(Cadence::Daily), &existing, None), Some(slot));
    }

    #[test]
    fn a_trigger_that_would_never_fire_at_that_moment_moves_the_slot() {
        // Saturday at nine, told to run on weekdays. It keeps the hour and
        // gives up the day, or the row says "every weekday" and fires on a
        // Saturday.
        let saturday = at(2025, 6, 14, 9, 0);
        let existing = routine(clock(Cadence::Daily), Some(saturday));
        let moved = next_slot_for(&clock(Cadence::Weekdays), &existing, None).unwrap();
        assert_ne!(moved, saturday);
        assert!(Cadence::Weekdays.accepts(moved), "it moved to a day it would actually fire on");
    }

    #[test]
    fn coming_back_to_the_clock_from_an_event_starts_like_a_new_routine() {
        // An event trigger holds no slot, so there is nothing to keep and
        // nothing to compute a next firing from.
        let existing = routine(event("stripe", "invoice.paid"), None);
        assert!(next_slot_for(&clock(Cadence::Daily), &existing, None).is_some());
        // And going the other way lands nowhere at all, whatever was asked for.
        let clocked = routine(clock(Cadence::Daily), Some(at(2025, 6, 10, 9, 0)));
        assert_eq!(next_slot_for(&event("stripe", "invoice.paid"), &clocked, Some(60)), None);
    }

    #[test]
    fn a_second_routine_for_work_already_standing_is_recognized_as_the_same_job() {
        // The failure this exists for: an operator asks for an adjustment to
        // something the agent already keeps, and the agent writes a second
        // routine beside the first. Both fire, and the work happens twice.
        assert!(same_job(
            "Check the new listings and email me a summary.",
            "Check Zillow listings and send me a summary of what is new.",
        ));
        // Reworded down to nothing recognizable in common is not the same job,
        // and neither is a routine that merely works on the same subject.
        assert!(!same_job(
            "Check the new listings and email me a summary.",
            "Write up this week's market activity and file it in the drive.",
        ));
        assert!(!same_job("Check the listings.", "Pay the invoices."));
    }

    #[test]
    fn two_instructions_of_pure_grammar_are_not_claimed_to_match() {
        // Nothing to compare is not evidence of a match. An empty subject set
        // divided into is also how this would have panicked.
        assert!(!same_job("do it now", "do it now"));
        assert!(!same_job("", "check the listings"));
    }
}
