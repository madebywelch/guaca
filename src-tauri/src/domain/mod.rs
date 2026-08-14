pub mod agent;
pub mod envelope;
pub mod group;
pub mod ids;

use std::time::{SystemTime, UNIX_EPOCH};

/// Milliseconds since the Unix epoch.
///
/// Saturates at 0 rather than panicking if the system clock is set before 1970.
/// A wrong timestamp is a cosmetic bug; a panic inside an agent actor takes the
/// agent down.
pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_is_after_2020_and_in_milliseconds() {
        // 2020-01-01T00:00:00Z in ms. Catches a seconds/millis mix-up, which is
        // otherwise invisible until timestamps sort wrong in the transcript.
        assert!(now_ms() > 1_577_836_800_000);
    }
}
