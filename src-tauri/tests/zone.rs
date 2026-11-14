//! The host's schedules in the operator's zone, once the host is given it.
//!
//! Its own binary because it sets `TZ` for the whole process: in the library's
//! tests it would move the local clock under every other test that reads it.
//! One test, so nothing reads the clock before the zone is set.

use chrono::{NaiveDate, NaiveTime, TimeZone, Utc};
use guac_lib::domain::decision::next_briefing;
use guac_lib::domain::instant;
use guac_lib::domain::routine::Cadence;

fn at(y: i32, m: u32, d: u32, hour: u32) -> i64 {
    let date = NaiveDate::from_ymd_opt(y, m, d).unwrap();
    instant(date.and_time(NaiveTime::from_hms_opt(hour, 0, 0).unwrap())).unwrap()
}

#[test]
fn a_host_given_new_york_keeps_new_york_hours() {
    std::env::set_var("TZ", "America/New_York");
    let utc = |y, m, d, h| Utc.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap().timestamp_millis();
    assert_eq!(at(2026, 11, 14, 9), utc(2026, 11, 14, 14), "the zone was not taken up");

    assert_eq!(next_briefing(at(2026, 11, 14, 8)), at(2026, 11, 14, 9));
    assert_eq!(next_briefing(at(2026, 11, 14, 10)), at(2026, 11, 14, 16));

    // 2026-11-13 is a Friday. At 21:00 in New York it is already Saturday in
    // UTC, which a host on UTC would have judged the weekday by.
    let friday = at(2026, 11, 13, 21);
    let next = Cadence::Weekdays.next_after(friday, friday + 1000).unwrap();
    assert_eq!(next, at(2026, 11, 16, 21), "Monday at 21:00 New York time");
}
