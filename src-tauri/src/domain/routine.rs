//! Routines: an agent's own schedule.
//!
//! An agent sets these for itself. "Check the listings every five hours" and
//! "wake me in an hour" are the same thing with and without a repeat, so both
//! are one row: what to do, and when it is next due.
//!
//! What is stored is the next due time rather than a running timer, so a
//! schedule survives a restart. Nothing has to be held in memory for a routine
//! to fire tomorrow.

use serde::{Deserialize, Serialize};

use super::ids::{AgentId, RoutineId};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Routine {
    pub id: RoutineId,
    pub agent_id: AgentId,
    /// The instruction the agent gave itself, delivered when it fires.
    pub what: String,
    /// `None` fires once and is done.
    pub every_secs: Option<u32>,
    pub next_run_at: i64,
    pub last_run_at: Option<i64>,
    pub created_at: i64,
}

impl Routine {
    pub fn repeats(&self) -> bool {
        self.every_secs.is_some()
    }

    /// When this should next fire, counted from the moment it just ran.
    ///
    /// Counted forward from now rather than from the previous due time, so a
    /// routine that was late, or that slept through its slot, does not then
    /// fire repeatedly to catch up.
    pub fn after_running(&self, now: i64) -> Option<i64> {
        self.every_secs.map(|secs| now + i64::from(secs) * 1000)
    }

    /// How it reads back to the agent that set it.
    pub fn describe(&self) -> String {
        match self.every_secs {
            Some(secs) => format!("every {}, next {}", human_gap(secs), when(self.next_run_at)),
            None => format!("once, {}", when(self.next_run_at)),
        }
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

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum RoutineError {
    #[error("a routine needs something to do")]
    Empty,
    #[error("the shortest repeat is {MIN_EVERY_SECS} seconds, got {got}")]
    TooOften { got: u32 },
    #[error("that is further ahead than this can schedule")]
    TooFar,
}

/// Checks what an agent asked for before it becomes a row.
pub fn validate(
    what: &str,
    every_secs: Option<u32>,
    in_secs: Option<u32>,
) -> Result<(), RoutineError> {
    if what.trim().is_empty() {
        return Err(RoutineError::Empty);
    }
    if let Some(every) = every_secs {
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
    fn a_repeat_is_counted_from_when_it_ran_not_from_when_it_was_due() {
        // A machine asleep through three slots must not wake and fire three
        // times to catch up.
        let routine = Routine {
            id: RoutineId::new(),
            agent_id: AgentId::new(),
            what: "check".into(),
            every_secs: Some(3600),
            next_run_at: 1_000,
            last_run_at: None,
            created_at: 0,
        };
        assert_eq!(routine.after_running(10_000_000), Some(10_000_000 + 3_600_000));
    }

    #[test]
    fn a_one_shot_has_no_next_time() {
        let routine = Routine {
            id: RoutineId::new(),
            agent_id: AgentId::new(),
            what: "wake me".into(),
            every_secs: None,
            next_run_at: 1_000,
            last_run_at: None,
            created_at: 0,
        };
        assert!(!routine.repeats());
        assert_eq!(routine.after_running(5_000), None);
    }

    #[test]
    fn a_routine_that_does_nothing_or_runs_constantly_is_refused() {
        assert_eq!(validate("  ", Some(3600), None), Err(RoutineError::Empty));
        assert_eq!(validate("x", Some(5), None), Err(RoutineError::TooOften { got: 5 }));
        assert_eq!(validate("x", None, Some(MAX_DELAY_SECS + 1)), Err(RoutineError::TooFar));
        assert_eq!(validate("x", Some(3600), Some(60)), Ok(()));
    }
}
