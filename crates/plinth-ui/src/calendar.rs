//! Pure calendar math for `DatePicker` (SPEC.md §6.3). No date crate: a
//! small, well-known algorithm (Howard Hinnant's `days_from_civil`) gives
//! the day-of-week, and a plain Gregorian leap-year rule gives the days in
//! a month. Kept separate from `render.rs` so it is easy to unit test.

/// True when `year` is a Gregorian leap year.
pub fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// The number of days in `month` (1..=12) of `year`.
pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => if is_leap(year) { 29 } else { 28 },
        _ => 30,
    }
}

/// Days since the epoch 1970-01-01 for a civil (Gregorian) date. Howard
/// Hinnant's `days_from_civil` algorithm, valid for all proleptic
/// Gregorian dates representable in `i64`.
fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year as i64 - 1 } else { year as i64 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as i64; // [0, 399]
    let mp = (month as i64 + 9) % 12; // [0, 11] Mar=0 .. Feb=11
    let doy = (153 * mp + 2) / 5 + day as i64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

/// The inverse of [`days_from_civil`]: the civil date `days` since the
/// epoch falls on.
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as i64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    (year as i32, m, d)
}

/// Today's date (local wall clock is not available in this `no_std`-free
/// crate, so this uses UTC, which is the host's best guess of "today"
/// without a timezone database).
pub fn today() -> (i32, u32, u32) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    civil_from_days((secs / 86_400) as i64)
}

/// The weekday of `year-month-day`: 0 = Monday .. 6 = Sunday (ISO order,
/// matching the Mo..Su header the picker shows).
pub fn weekday_of(year: i32, month: u32, day: u32) -> u32 {
    let days = days_from_civil(year, month, day);
    // 1970-01-01 was a Thursday (ISO weekday 3, 0-based Mon=0).
    (((days + 3) % 7 + 7) % 7) as u32
}

/// Adds `delta` months to `(year, month)`, normalizing the month back into
/// `1..=12` and carrying into `year`.
pub fn add_months(year: i32, month: u32, delta: i32) -> (i32, u32) {
    let zero_based = month as i32 - 1 + delta;
    let y = year + zero_based.div_euclid(12);
    let m = zero_based.rem_euclid(12) + 1;
    (y, m as u32)
}

/// Clamps `day` to a valid day of `(year, month)`.
pub fn clamp_day(year: i32, month: u32, day: u32) -> u32 {
    day.clamp(1, days_in_month(year, month))
}

/// Parses `"YYYY-MM-DD"` into `(year, month, day)`. `None` for anything
/// else, including the empty string (SPEC.md §6.3: no value).
pub fn parse_date(s: &str) -> Option<(i32, u32, u32)> {
    let mut it = s.split('-');
    let y = it.next()?.parse::<i32>().ok()?;
    let m = it.next()?.parse::<u32>().ok()?;
    let d = it.next()?.parse::<u32>().ok()?;
    if it.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some((y, m, d))
}

/// Parses `"HH:MM"` into `(hour, minute)`.
pub fn parse_time(s: &str) -> Option<(u32, u32)> {
    let mut it = s.split(':');
    let h = it.next()?.parse::<u32>().ok()?;
    let m = it.next()?.parse::<u32>().ok()?;
    if it.next().is_some() || h > 23 || m > 59 {
        return None;
    }
    Some((h, m))
}

/// Splits an ISO `DatePicker` value into its date and time parts, following
/// `mode`: `"date"` -> date only, `"time"` -> time only, `"datetime"` ->
/// `"YYYY-MM-DDTHH:MM"` split on `T`.
pub fn parse_value(mode: &str, value: &str) -> (Option<(i32, u32, u32)>, Option<(u32, u32)>) {
    if value.is_empty() {
        return (None, None);
    }
    match mode {
        "time" => (None, parse_time(value)),
        "datetime" => match value.split_once('T') {
            Some((d, t)) => (parse_date(d), parse_time(t)),
            None => (parse_date(value), None),
        },
        _ => (parse_date(value), None),
    }
}

/// Composes an ISO value from parts, following `mode`.
pub fn format_value(mode: &str, date: Option<(i32, u32, u32)>, time: Option<(u32, u32)>) -> String {
    let d = date.map(|(y, m, day)| format!("{y:04}-{m:02}-{day:02}"));
    let t = time.map(|(h, m)| format!("{h:02}:{m:02}"));
    match mode {
        "time" => t.unwrap_or_default(),
        "datetime" => match (d, t) {
            (Some(d), Some(t)) => format!("{d}T{t}"),
            (Some(d), None) => d,
            (None, Some(t)) => t,
            (None, None) => String::new(),
        },
        _ => d.unwrap_or_default(),
    }
}

const MONTH_NAMES: [&str; 12] =
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

pub fn month_name(month: u32) -> &'static str {
    MONTH_NAMES.get((month.wrapping_sub(1)) as usize).copied().unwrap_or("?")
}

/// A locale-neutral English reading of a date, e.g. "Oct 6, 2026".
pub fn format_date_readable(year: i32, month: u32, day: u32) -> String {
    format!("{} {day}, {year}", month_name(month))
}

/// A 12-hour reading of a time, e.g. "2:30 PM".
pub fn format_time_readable(hour: u32, minute: u32) -> String {
    let (h12, suffix) = match hour {
        0 => (12, "AM"),
        1..=11 => (hour, "AM"),
        12 => (12, "PM"),
        _ => (hour - 12, "PM"),
    };
    format!("{h12}:{minute:02} {suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leap_years() {
        assert!(is_leap(2000));
        assert!(is_leap(2024));
        assert!(!is_leap(1900));
        assert!(!is_leap(2023));
    }

    #[test]
    fn days_in_month_incl_leap() {
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2023, 2), 28);
        assert_eq!(days_in_month(2026, 4), 30);
        assert_eq!(days_in_month(2026, 1), 31);
    }

    #[test]
    fn weekday_known_dates() {
        // 1970-01-01 was a Thursday -> 3 (Mon=0).
        assert_eq!(weekday_of(1970, 1, 1), 3);
        // 2000-01-01 was a Saturday -> 5.
        assert_eq!(weekday_of(2000, 1, 1), 5);
        // 2026-10-06 (today, per the task) is a Tuesday -> 1.
        assert_eq!(weekday_of(2026, 10, 6), 1);
    }

    #[test]
    fn add_months_wraps_year() {
        assert_eq!(add_months(2026, 12, 1), (2027, 1));
        assert_eq!(add_months(2026, 1, -1), (2025, 12));
        assert_eq!(add_months(2026, 6, 12), (2027, 6));
        assert_eq!(add_months(2026, 6, -18), (2024, 12));
    }

    #[test]
    fn clamp_day_to_month_length() {
        assert_eq!(clamp_day(2023, 2, 30), 28);
        assert_eq!(clamp_day(2024, 2, 30), 29);
        assert_eq!(clamp_day(2026, 1, 15), 15);
    }

    #[test]
    fn parses_and_formats_date() {
        assert_eq!(parse_date("2026-10-06"), Some((2026, 10, 6)));
        assert_eq!(parse_date(""), None);
        assert_eq!(parse_date("not-a-date"), None);
        assert_eq!(format_date_readable(2026, 10, 6), "Oct 6, 2026");
    }

    #[test]
    fn parses_and_formats_time() {
        assert_eq!(parse_time("14:30"), Some((14, 30)));
        assert_eq!(parse_time("25:00"), None);
        assert_eq!(format_time_readable(14, 30), "2:30 PM");
        assert_eq!(format_time_readable(0, 5), "12:05 AM");
        assert_eq!(format_time_readable(12, 0), "12:00 PM");
    }

    #[test]
    fn civil_days_round_trip() {
        for (y, m, d) in [(1970, 1, 1), (2000, 2, 29), (2026, 10, 6), (1900, 3, 1), (2400, 2, 29)] {
            let days = days_from_civil(y, m, d);
            assert_eq!(civil_from_days(days), (y, m, d), "round trip for {y}-{m}-{d}");
        }
    }

    #[test]
    fn value_round_trips_by_mode() {
        assert_eq!(parse_value("date", "2026-10-06"), (Some((2026, 10, 6)), None));
        assert_eq!(format_value("date", Some((2026, 10, 6)), None), "2026-10-06");
        assert_eq!(parse_value("time", "14:30"), (None, Some((14, 30))));
        assert_eq!(format_value("time", None, Some((14, 30))), "14:30");
        assert_eq!(parse_value("datetime", "2026-10-06T14:30"), (Some((2026, 10, 6)), Some((14, 30))));
        assert_eq!(format_value("datetime", Some((2026, 10, 6)), Some((14, 30))), "2026-10-06T14:30");
        assert_eq!(parse_value("date", ""), (None, None));
    }
}
