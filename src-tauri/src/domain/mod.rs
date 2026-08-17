pub mod agent;
pub mod approval;
pub mod attachment;
pub mod connector;
pub mod envelope;
pub mod group;
pub mod ids;
pub mod routine;
pub mod signin;
pub mod usage;

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

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
        assert!(stamps.windows(2).all(|w| w[1] > w[0]), "and they must only ever go forwards");
    }

    #[test]
    fn now_is_after_2020_and_in_milliseconds() {
        // 2020-01-01T00:00:00Z in ms. Catches a seconds/millis mix-up, which is
        // otherwise invisible until timestamps sort wrong in the transcript.
        assert!(now_ms() > 1_577_836_800_000);
    }
}
