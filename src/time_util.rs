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
}
