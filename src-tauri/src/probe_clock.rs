//! How long the two waits the outbox measures absence in last on this Host.
//!
//! A person counts as away two minutes after a window last had focus, and
//! finished work is gathered for ten minutes before it is offered. Those are
//! the right lengths for a person and the wrong ones for `scripts/probe.py`,
//! which checks the whole path on a Mac and would wait twelve minutes a run.
//! A Host built with the `probe-host` feature and started with
//! [`MINUTE_ENV`] runs both on a shorter minute. Every other build compiles
//! [`scaled`] to the identity, and `probe-host` refuses to compile with
//! optimizations on, so no Host anyone ships can be told to hurry.

/// Milliseconds a minute lasts on a `probe-host` Host.
pub const MINUTE_ENV: &str = "GUACA_PROBE_MINUTE_MS";

#[cfg(all(feature = "probe-host", not(debug_assertions)))]
compile_error!(
    "the probe-host feature is for test builds only; build it with --debug, never for release"
);

/// `ms` as this Host counts it.
#[cfg(not(feature = "probe-host"))]
#[inline(always)]
pub const fn scaled(ms: i64) -> i64 {
    ms
}

/// `ms` as this Host counts it: unchanged unless the launch set
/// [`MINUTE_ENV`] to a minute between one second and one minute long.
#[cfg(feature = "probe-host")]
pub fn scaled(ms: i64) -> i64 {
    static MINUTE: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    let minute = *MINUTE.get_or_init(|| {
        std::env::var(MINUTE_ENV)
            .ok()
            .and_then(|value| value.trim().parse::<i64>().ok())
            .map_or(60_000, |value| value.clamp(1_000, 60_000))
    });
    ms * minute / 60_000
}

#[cfg(all(test, not(feature = "probe-host")))]
mod tests {
    #[test]
    fn without_the_probe_host_feature_a_minute_is_a_minute() {
        assert_eq!(super::scaled(120_000), 120_000);
        assert_eq!(super::scaled(10 * 60_000), 10 * 60_000);
    }
}
