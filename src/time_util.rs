//! Dependency-free UTC timestamp formatting (RFC 3339).
//!
//! Avoids a datetime crate by formatting `std::time::SystemTime` via Howard
//! Hinnant's `civil_from_days` algorithm. The only callers are controllers
//! stamping CRD `lastUpdated`; the webhook never reads a clock on the hot path
//! (the freshness check arrives in a later phase).

use std::time::{SystemTime, UNIX_EPOCH};

/// Current UTC time as an RFC 3339 string, e.g. `2026-07-26T14:32:05Z`.
///
/// Returns the epoch on a clock failure rather than panicking; a controller must
/// never crash the process over a timestamp.
pub fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    rfc3339_from_unix(secs as i64)
}

/// Current UTC time as Unix seconds. The webhook reads the clock exactly once
/// per admission request (to compute `freshness_seconds` and enforce the
/// freshness threshold); this is the single clock-touch on the hot path.
pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Parse an RFC 3339 UTC timestamp (`YYYY-MM-DDTHH:MM:SSZ`) into Unix seconds.
///
/// Returns `None` for anything that is not exactly this shape, including
/// timezone offsets — the controllers only ever write the `Z` form. An
/// unparseable `lastUpdated` is treated as stale upstream (Principle I:
/// fail-closed), so this signals failure rather than guessing.
pub fn parse_rfc3339(input: &str) -> Option<i64> {
    // Shape: "YYYY-MM-DDTHH:MM:SSZ". Split on the fixed delimiters; each field
    // must be present and in range, otherwise None (fail-closed upstream).
    let body = input.strip_suffix('Z')?;
    let (date, time) = body.split_once('T')?;
    let (year_s, month_day) = date.split_once('-')?;
    let (month_s, day_s) = month_day.split_once('-')?;
    let (hour_s, minute_second) = time.split_once(':')?;
    let (minute_s, second_s) = minute_second.split_once(':')?;

    let year: i64 = year_s.parse().ok()?;
    let month: i64 = month_s.parse().ok()?;
    let day: i64 = day_s.parse().ok()?;
    let hour: i64 = hour_s.parse().ok()?;
    let minute: i64 = minute_s.parse().ok()?;
    let second: i64 = second_s.parse().ok()?;

    if !(1..=9999).contains(&year)
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=59).contains(&second)
    {
        return None;
    }

    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3600 + minute * 60 + second)
}

/// Format a Unix timestamp (seconds since the epoch, UTC) as RFC 3339.
pub fn rfc3339_from_unix(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let day_secs = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = day_secs / 3600;
    let minute = (day_secs % 3600) / 60;
    let second = day_secs % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Days since 1970-01-01 → `(year, month, day)` (proleptic Gregorian, UTC).
/// Howard Hinnant, "civil_from_days".
#[allow(clippy::arithmetic_side_effects)]
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Inverse of [`civil_from_days`]: `(year, month, day)` → days since 1970-01-01.
/// Howard Hinnant, "days_from_civil". Used to parse an RFC 3339 date back to
/// Unix seconds for the freshness check.
#[allow(clippy::arithmetic_side_effects)]
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_is_1970_01_01() {
        assert_eq!(rfc3339_from_unix(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn one_day_is_jan_2_1970() {
        assert_eq!(rfc3339_from_unix(86_400), "1970-01-02T00:00:00Z");
    }

    #[test]
    fn anchor_2024_01_01_is_day_19723() {
        // 54 years (1970..2023) × 365 + 13 leap days = 19723.
        assert_eq!(rfc3339_from_unix(19_723 * 86_400), "2024-01-01T00:00:00Z");
    }

    #[test]
    fn leap_day_2024_02_29_formats_correctly() {
        // 2024-01-01 + 31 (Jan) + 28 days = 2024-02-29 (leap year path).
        assert_eq!(
            rfc3339_from_unix((19_723 + 59) * 86_400),
            "2024-02-29T00:00:00Z"
        );
    }

    #[test]
    fn day_after_leap_day_is_march_1() {
        assert_eq!(
            rfc3339_from_unix((19_723 + 60) * 86_400),
            "2024-03-01T00:00:00Z"
        );
    }

    #[test]
    fn now_is_well_formed() {
        let s = now_rfc3339();
        assert_eq!(s.len(), 20, "YYYY-MM-DDTHH:MM:SSZ is 20 chars");
        assert!(s.ends_with('Z'));
    }

    #[test]
    fn negative_seconds_before_epoch_floor_correctly() {
        // -1 second → 1969-12-31T23:59:59Z (div_euclid/rem_euclid handle negatives).
        assert_eq!(rfc3339_from_unix(-1), "1969-12-31T23:59:59Z");
    }

    // ---- parse_rfc3339 / now_unix (freshness check + freshness_seconds logging) ----

    #[test]
    fn parse_round_trips_format_output() {
        for secs in [0i64, 1, 86_400, 19_723 * 86_400, 1_784_747_525, -1] {
            let formatted = rfc3339_from_unix(secs);
            assert_eq!(parse_rfc3339(&formatted), Some(secs));
        }
    }

    #[test]
    fn parse_known_anchor() {
        // 2026-07-26T14:32:05Z (the spec fixture timestamp).
        assert_eq!(parse_rfc3339("2026-07-26T14:32:05Z"), Some(1_785_076_325));
    }

    #[test]
    fn parse_rejects_malformed() {
        assert_eq!(parse_rfc3339(""), None);
        assert_eq!(parse_rfc3339("not-a-date"), None);
        assert_eq!(parse_rfc3339("2026-07-26"), None); // missing time
        assert_eq!(parse_rfc3339("2026-13-40T99:99:99Z"), None); // out of range
    }

    #[test]
    fn now_unix_is_positive_and_round_trips() {
        let secs = now_unix();
        assert!(secs > 0, "current time must be after the epoch");
        assert_eq!(parse_rfc3339(&rfc3339_from_unix(secs)), Some(secs));
    }
}
