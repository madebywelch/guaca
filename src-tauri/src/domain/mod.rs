pub mod agent;
pub mod approval;
pub mod attachment;
pub mod connector;
pub mod envelope;
pub mod escalation;
pub mod group;
pub mod ids;
pub mod occasion;
pub mod plugin;
pub mod promise;
pub mod repository;
pub mod routine;
pub mod search;
pub mod signin;
pub mod usage;
pub mod worknote;

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Duration, Local, LocalResult, NaiveDateTime, TimeZone};

/// Milliseconds since the Unix epoch.
///
/// Saturates at 0 rather than panicking if the system clock is set before 1970.
/// A wrong timestamp is a cosmetic bug; a panic inside an agent actor takes the
/// agent down.
pub fn now_ms() -> i64 {
    // Strictly increasing, not merely non-decreasing. A transcript is ordered by
    // this, and two records written inside the same millisecond used to tie-break
    // on a random uuid: an agent's tool trail could be drawn after the reply it
    // came before, so "Manager used run_command" appeared under Manager's answer.
    static LAST: AtomicI64 = AtomicI64::new(0);
    let wall =
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    LAST.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |prev| Some(wall.max(prev + 1)))
        .map(|prev| wall.max(prev + 1))
        .unwrap_or(wall)
}

/// The local wall-clock moment a timestamp lands on.
///
/// Here rather than in one of the two modules that need it, beside `now_ms`,
/// because it is the same kind of fact: this app stores instants and shows
/// people wall clocks, and the conversion between the two has one answer. A
/// schedule and a calendar both cross that line and neither owns it.
pub fn local(ms: i64) -> Option<DateTime<Local>> {
    DateTime::from_timestamp_millis(ms).map(|utc| utc.with_timezone(&Local))
}

/// The instant a local wall-clock time happens at.
///
/// Both DST edges are handled here rather than left to the caller. Springing
/// forward deletes an hour, so a moment anchored inside it does not exist that
/// day and takes the next hour; falling back doubles one, and the first pass is
/// the one that was meant.
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

/// Trims a string and cuts it to `max` characters, on a word where it can.
///
/// Characters rather than bytes, which is not a detail: a line of emoji is a
/// quarter of the bytes it looks like and must not be cut four times as early.
/// The flag is what makes this honest — every caller hands it back to whoever
/// wrote the text, because an agent that believes it recorded something it did
/// not will not write it again.
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
        // Written back to back, these land in one millisecond. Equal stamps left
        // the order to a uuid tie-break, which put a turn's tool calls below the
        // answer they produced.
        let stamps: Vec<i64> = (0..500).map(|_| now_ms()).collect();
        let mut sorted = stamps.clone();
        sorted.dedup();
        assert_eq!(stamps.len(), sorted.len(), "two records must never share a timestamp");
        assert!(stamps.windows(2).all(|w| w[1] > w[0]), "and they must only ever go forward");
    }

    #[test]
    fn now_is_after_2020_and_in_milliseconds() {
        // 2020-01-01T00:00:00Z in ms. Catches a seconds/millis mix-up, which is
        // otherwise invisible until timestamps sort wrong in the transcript.
        assert!(now_ms() > 1_577_836_800_000);
    }
}
