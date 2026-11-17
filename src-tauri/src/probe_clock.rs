pub const MINUTE_ENV: &str = "GUACA_PROBE_MINUTE_MS";

#[cfg(all(feature = "probe-host", not(debug_assertions)))]
compile_error!(
    "the probe-host feature is for test builds only; build it with --debug, never for release"
);

#[cfg(not(feature = "probe-host"))]
#[inline(always)]
pub const fn scaled(ms: i64) -> i64 {
    ms
}

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
