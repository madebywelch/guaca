pub mod agent;
pub mod approval;
pub mod artifact;
pub mod attachment;
pub mod connector;
pub mod decision;
pub mod deployment;
pub mod effort;
pub mod envelope;
pub mod errand;
pub mod escalation;
pub mod group;
pub mod ids;
pub mod occasion;
pub mod outbox;
pub mod outcome;
pub mod plugin;
pub mod policy;
pub mod promise;
pub mod quick;
pub mod routine;
pub mod search;
pub mod signin;
pub mod skill;
pub mod usage;
pub mod view;
pub mod widget;
pub mod worknote;

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Duration, Local, LocalResult, NaiveDateTime, TimeZone};

pub fn now_ms() -> i64 {
    static LAST: AtomicI64 = AtomicI64::new(0);
    let wall =
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    LAST.try_update(Ordering::SeqCst, Ordering::SeqCst, |prev| Some(wall.max(prev + 1)))
        .map(|prev| wall.max(prev + 1))
        .unwrap_or(wall)
}

pub fn local(ms: i64) -> Option<DateTime<Local>> {
    DateTime::from_timestamp_millis(ms).map(|utc| utc.with_timezone(&Local))
}

pub fn instant(naive: NaiveDateTime) -> Option<i64> {
    match Local.from_local_datetime(&naive) {
        LocalResult::Single(at) => Some(at.timestamp_millis()),
        LocalResult::Ambiguous(first, _) => Some(first.timestamp_millis()),
        LocalResult::None => {
            let shifted = naive.checked_add_signed(Duration::hours(1))?;
            Local.from_local_datetime(&shifted).earliest().map(|at| at.timestamp_millis())
        }
    }
}

pub fn cut_to(body: &str, max: usize) -> (String, bool) {
    let trimmed = body.trim();
    if trimmed.chars().count() <= max {
        return (trimmed.to_string(), false);
    }
    let mut kept: String = trimmed.chars().take(max).collect();
    if let Some(space) = kept.rfind(char::is_whitespace) {
        if space > max / 2 {
            kept.truncate(space);
        }
    }
    (kept.trim_end().to_string(), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_never_repeat_so_a_transcript_keeps_its_order() {
        let stamps: Vec<i64> = (0..500).map(|_| now_ms()).collect();
        let mut sorted = stamps.clone();
        sorted.dedup();
        assert_eq!(stamps.len(), sorted.len(), "two records must never share a timestamp");
        assert!(stamps.windows(2).all(|w| w[1] > w[0]), "and they must only ever go forward");
    }

    #[test]
    fn now_is_after_2020_and_in_milliseconds() {
        assert!(now_ms() > 1_577_836_800_000);
    }
}
