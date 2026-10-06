//! Date/time math for `plinth:time` (SPEC.md §8.5, docs/GAPS.md gap #5):
//! `dateParts`, `makeDate`, `formatDate`, `parseDate`, `toISOString`.
//!
//! No date crate (SPEC.md §5.5: `plinth-rt` stays `no_std`-small): the
//! civil-date math is Howard Hinnant's `days_from_civil`/`civil_from_days`
//! algorithm (same one `plinth-ui/src/calendar.rs` uses for `DatePicker`),
//! and string building uses `alloc::string::String` with manual digit
//! pushing, like `strings.rs` (no `core::fmt`/`format!`).
//!
//! Every function here is pure: the local time zone offset (minutes east
//! of UTC at a given instant) is passed in rather than fetched, so this
//! module is fully unit-testable without a host. The ABI wrappers in
//! `lib.rs` supply the real offset via `host::timezone_offset`.

use alloc::string::String;

const MS_PER_DAY: i64 = 86_400_000;

/// Days since the epoch 1970-01-01 for a civil (Gregorian) date. Howard
/// Hinnant's `days_from_civil` algorithm, valid for all proleptic
/// Gregorian dates representable in `i64`.
fn days_from_civil(year: i32, month: i32, day: i32) -> i64 {
    let y = if month <= 2 { year as i64 - 1 } else { year as i64 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = (month as i64 + 9) % 12; // [0, 11] Mar=0 .. Feb=11
    let doy = (153 * mp + 2) / 5 + day as i64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

/// The inverse of [`days_from_civil`].
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    (year as i32, m, d)
}

/// Floor division (`a.div_euclid` for the day/ms split, since `ms` can be
/// negative for pre-1970 instants).
fn floor_div(a: i64, b: i64) -> i64 {
    let q = a / b;
    let r = a % b;
    if r != 0 && ((r < 0) != (b < 0)) { q - 1 } else { q }
}

fn floor_mod(a: i64, b: i64) -> i64 {
    a - floor_div(a, b) * b
}

/// A decomposed instant, SPEC.md's `DateParts`. `weekday`: 0 = Sunday.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parts {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub millisecond: u32,
    pub weekday: u32,
}

/// Decomposes `effective_ms` (already shifted by the local offset, or the
/// raw UTC ms when the caller wants UTC fields) into its calendar parts.
pub fn decompose(effective_ms: i64) -> Parts {
    let days = floor_div(effective_ms, MS_PER_DAY);
    let rem = floor_mod(effective_ms, MS_PER_DAY) as u32;
    let (year, month, day) = civil_from_days(days);
    let weekday = floor_mod(days + 4, 7) as u32; // 1970-01-01 was a Thursday.
    Parts {
        year,
        month,
        day,
        hour: rem / 3_600_000,
        minute: (rem / 60_000) % 60,
        second: (rem / 1_000) % 60,
        millisecond: rem % 1_000,
        weekday,
    }
}

/// `dateParts(ms, utc)`'s field selection, by index (matches `date_field`'s
/// ABI `field` argument: 0=year .. 7=weekday).
pub fn field(ms: f64, utc: bool, offset_minutes_at: impl Fn(i64) -> i32, field_index: i32) -> f64 {
    let ms = ms as i64;
    let offset = if utc { 0 } else { offset_minutes_at(ms) as i64 };
    let p = decompose(ms + offset * 60_000);
    (match field_index {
        0 => p.year,
        1 => p.month as i32,
        2 => p.day as i32,
        3 => p.hour as i32,
        4 => p.minute as i32,
        5 => p.second as i32,
        6 => p.millisecond as i32,
        _ => p.weekday as i32,
    }) as f64
}

/// `makeDate(year, month, day, hour, minute, second)`: local wall-clock
/// components to ms since the epoch. The offset is looked up at the naive
/// (as-if-UTC) instant, which matches what hosts report for that wall
/// clock date in all but the rare case of a date inside a DST transition.
pub fn make_date(year: f64, month: f64, day: f64, hour: f64, minute: f64, second: f64, offset_minutes_at: impl Fn(i64) -> i32) -> f64 {
    let naive = days_from_civil(year as i32, month as i32, day as i32) * MS_PER_DAY
        + (hour as i64) * 3_600_000
        + (minute as i64) * 60_000
        + (second as i64) * 1_000;
    let offset = offset_minutes_at(naive) as i64;
    (naive - offset * 60_000) as f64
}

const WEEKDAY_SHORT: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const WEEKDAY_LONG: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTH_SHORT: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const MONTH_LONG: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

fn push_pad(out: &mut String, mut v: u32, width: usize) {
    let mut digits = [0u8; 10];
    let mut i = digits.len();
    loop {
        i -= 1;
        digits[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    let n = digits.len() - i;
    for _ in n..width {
        out.push('0');
    }
    out.push_str(core::str::from_utf8(&digits[i..]).unwrap());
}

/// `formatDate(ms, pattern, utc)`. Pattern tokens: `YYYY MM M DD D HH H mm
/// ss SSS ddd dddd MMM MMMM A`; everything else passes through literally.
/// English names only (SPEC.md §4.7 `plinth:locale` is where locale-aware
/// formatting belongs later).
pub fn format_date(ms: f64, pattern: &str, utc: bool, offset_minutes_at: impl Fn(i64) -> i32) -> String {
    let ms_i = ms as i64;
    let offset = if utc { 0 } else { offset_minutes_at(ms_i) as i64 };
    let p = decompose(ms_i + offset * 60_000);
    let hour12 = match p.hour % 12 {
        0 => 12,
        h => h,
    };
    let bytes = pattern.as_bytes();
    let mut out = String::with_capacity(pattern.len() + 8);
    let mut i = 0;
    // Longest tokens first so e.g. "YYYY" is not read as "Y" + "YYY".
    let tokens: &[(&str, &dyn Fn(&mut String))] = &[
        ("YYYY", &|o: &mut String| push_pad(o, p.year.unsigned_abs(), 4)),
        ("MMMM", &|o: &mut String| o.push_str(MONTH_LONG[(p.month - 1) as usize])),
        ("MMM", &|o: &mut String| o.push_str(MONTH_SHORT[(p.month - 1) as usize])),
        ("MM", &|o: &mut String| push_pad(o, p.month, 2)),
        ("M", &|o: &mut String| push_pad(o, p.month, 1)),
        ("DD", &|o: &mut String| push_pad(o, p.day, 2)),
        ("D", &|o: &mut String| push_pad(o, p.day, 1)),
        ("HH", &|o: &mut String| push_pad(o, p.hour, 2)),
        ("H", &|o: &mut String| push_pad(o, p.hour, 1)),
        ("hh", &|o: &mut String| push_pad(o, hour12, 2)),
        ("h", &|o: &mut String| push_pad(o, hour12, 1)),
        ("mm", &|o: &mut String| push_pad(o, p.minute, 2)),
        ("ss", &|o: &mut String| push_pad(o, p.second, 2)),
        ("SSS", &|o: &mut String| push_pad(o, p.millisecond, 3)),
        ("dddd", &|o: &mut String| o.push_str(WEEKDAY_LONG[p.weekday as usize])),
        ("ddd", &|o: &mut String| o.push_str(WEEKDAY_SHORT[p.weekday as usize])),
        ("A", &|o: &mut String| o.push_str(if p.hour < 12 { "AM" } else { "PM" })),
    ];
    'outer: while i < bytes.len() {
        for (tok, f) in tokens {
            if pattern[i..].starts_with(tok) {
                f(&mut out);
                i += tok.len();
                continue 'outer;
            }
        }
        // Copy one UTF-8 scalar literally.
        let ch = pattern[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// `toISOString(ms)`: UTC, like JS's `Date.prototype.toISOString`. Not
/// called by the ABI (the compiler composes it from `format_date` plus a
/// literal `"Z"`, SPEC.md §4.7, to avoid a dedicated runtime function); kept
/// here so the exact format is unit-tested against this module directly.
#[cfg_attr(not(test), allow(dead_code))]
pub fn to_iso_string(ms: f64) -> String {
    let mut s = format_date(ms, "YYYY-MM-DDTHH:mm:ss.SSS", true, |_| 0);
    s.push('Z');
    s
}

/// `parseDate(text)`. Accepts `YYYY-MM-DD` and `YYYY-MM-DDTHH:MM[:SS[.sss]]
/// [Z|±HH:MM]`. No `Z`/offset means local time (resolved via
/// `offset_minutes_at`, queried at the naive instant). Returns `None` for
/// anything else.
pub fn parse_date(text: &str, offset_minutes_at: impl Fn(i64) -> i32) -> Option<f64> {
    let b = text.as_bytes();
    let digits = |s: &[u8]| -> Option<i64> {
        if s.is_empty() || !s.iter().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let mut v = 0i64;
        for &c in s {
            v = v * 10 + (c - b'0') as i64;
        }
        Some(v)
    };
    // YYYY-MM-DD, at minimum.
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let year = digits(&b[0..4])? as i32;
    let month = digits(&b[5..7])?;
    let day = digits(&b[8..10])?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if text.len() == 10 {
        let naive = days_from_civil(year, month as i32, day as i32) * MS_PER_DAY;
        let offset = offset_minutes_at(naive) as i64;
        return Some((naive - offset * 60_000) as f64);
    }
    if b.len() < 16 || (b[10] != b'T' && b[10] != b' ') || b[13] != b':' {
        return None;
    }
    let hour = digits(&b[11..13])?;
    let minute = digits(&b[14..16])?;
    if hour > 23 || minute > 59 {
        return None;
    }
    let mut pos = 16;
    let mut second = 0i64;
    let mut millis = 0i64;
    if pos < b.len() && b[pos] == b':' {
        if pos + 3 > b.len() {
            return None;
        }
        second = digits(&b[pos + 1..pos + 3])?;
        if second > 59 {
            return None;
        }
        pos += 3;
        if pos < b.len() && b[pos] == b'.' {
            let start = pos + 1;
            let mut end = start;
            while end < b.len() && b[end].is_ascii_digit() {
                end += 1;
            }
            if end == start {
                return None;
            }
            let frac = digits(&b[start..end])?;
            let width = end - start;
            // Normalize to milliseconds regardless of fraction width.
            millis = match width {
                1 => frac * 100,
                2 => frac * 10,
                3 => frac,
                _ => frac / 10i64.pow((width - 3) as u32),
            };
            pos = end;
        }
    }
    let naive = days_from_civil(year, month as i32, day as i32) * MS_PER_DAY
        + hour * 3_600_000
        + minute * 60_000
        + second * 1_000
        + millis;
    if pos == b.len() {
        // No `Z`/offset: local time.
        let offset = offset_minutes_at(naive) as i64;
        return Some((naive - offset * 60_000) as f64);
    }
    if b[pos] == b'Z' && pos + 1 == b.len() {
        return Some(naive as f64);
    }
    if (b[pos] == b'+' || b[pos] == b'-') && b.len() == pos + 6 && b[pos + 3] == b':' {
        let sign = if b[pos] == b'-' { -1 } else { 1 };
        let oh = digits(&b[pos + 1..pos + 3])?;
        let om = digits(&b[pos + 4..pos + 6])?;
        let explicit_offset = sign * (oh * 60 + om);
        return Some((naive - explicit_offset * 60_000) as f64);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = MS_PER_DAY;

    #[test]
    fn round_trip_civil() {
        for (y, m, d) in [(1970, 1, 1), (2000, 2, 29), (2100, 2, 28), (1969, 12, 31), (1600, 1, 1), (2024, 12, 31)] {
            let days = days_from_civil(y, m, d);
            assert_eq!(civil_from_days(days), (y, m as u32, d as u32), "{y}-{m}-{d}");
        }
    }

    #[test]
    fn decompose_fixed_epochs() {
        // Epoch.
        let p = decompose(0);
        assert_eq!((p.year, p.month, p.day, p.hour, p.minute, p.second, p.weekday), (1970, 1, 1, 0, 0, 0, 4));
        // Leap day 2000-02-29 12:34:56.789.
        let ms = days_from_civil(2000, 2, 29) * DAY + 12 * 3_600_000 + 34 * 60_000 + 56_000 + 789;
        let p = decompose(ms);
        assert_eq!((p.year, p.month, p.day, p.hour, p.minute, p.second, p.millisecond), (2000, 2, 29, 12, 34, 56, 789));
        // 2100 is not a leap year: Feb has 28 days, so day 29 rolls to March 1.
        let ms = days_from_civil(2100, 2, 28) * DAY;
        assert_eq!(decompose(ms).day, 28);
        assert_eq!(civil_from_days(days_from_civil(2100, 2, 28) + 1), (2100, 3, 1));
        // End of month.
        let p = decompose(days_from_civil(2024, 1, 31) * DAY);
        assert_eq!((p.year, p.month, p.day), (2024, 1, 31));
        // Negative timestamp before 1970.
        let p = decompose(-DAY); // 1969-12-31
        assert_eq!((p.year, p.month, p.day, p.hour, p.weekday), (1969, 12, 31, 0, 3));
        let p = decompose(-1); // 1969-12-31 23:59:59.999
        assert_eq!((p.year, p.month, p.day, p.hour, p.minute, p.second, p.millisecond), (1969, 12, 31, 23, 59, 59, 999));
    }

    #[test]
    fn weekday_matches_known_dates() {
        // 1970-01-01 Thursday, 2000-01-01 Saturday, 2024-01-01 Monday.
        assert_eq!(decompose(days_from_civil(1970, 1, 1) * DAY).weekday, 4);
        assert_eq!(decompose(days_from_civil(2000, 1, 1) * DAY).weekday, 6);
        assert_eq!(decompose(days_from_civil(2024, 1, 1) * DAY).weekday, 1);
    }

    #[test]
    fn field_utc_matches_decompose() {
        let ms = (days_from_civil(2024, 3, 15) * DAY + 9 * 3_600_000) as f64;
        assert_eq!(field(ms, true, |_| 0, 0), 2024.0);
        assert_eq!(field(ms, true, |_| 0, 1), 3.0);
        assert_eq!(field(ms, true, |_| 0, 2), 15.0);
        assert_eq!(field(ms, true, |_| 0, 3), 9.0);
    }

    #[test]
    fn field_applies_local_offset() {
        // 2024-03-15T00:30:00Z with offset -300 (US Eastern) is still
        // 2024-03-14 locally.
        let ms = (days_from_civil(2024, 3, 15) * DAY + 30 * 60_000) as f64;
        assert_eq!(field(ms, false, |_| -300, 2), 14.0);
        assert_eq!(field(ms, false, |_| -300, 3), 19.0); // 24 - 5 = 19:30
    }

    #[test]
    fn make_date_round_trips_with_field() {
        let ms = make_date(2024.0, 3.0, 15.0, 9.0, 30.0, 0.0, |_| -300);
        assert_eq!(field(ms, false, |_| -300, 0), 2024.0);
        assert_eq!(field(ms, false, |_| -300, 1), 3.0);
        assert_eq!(field(ms, false, |_| -300, 2), 15.0);
        assert_eq!(field(ms, false, |_| -300, 3), 9.0);
        assert_eq!(field(ms, false, |_| -300, 4), 30.0);
        // UTC is 5 hours ahead.
        assert_eq!(field(ms, true, |_| 0, 3), 14.0);
    }

    #[test]
    fn format_utc_patterns() {
        let ms = (days_from_civil(2024, 3, 5) * DAY + 9 * 3_600_000 + 7 * 60_000 + 3_000 + 45) as f64;
        assert_eq!(format_date(ms, "YYYY-MM-DD HH:mm:ss.SSS", true, |_| 0), "2024-03-05 09:07:03.045");
        assert_eq!(format_date(ms, "D/M/YYYY", true, |_| 0), "5/3/2024");
        assert_eq!(format_date(ms, "dddd, MMMM D, YYYY", true, |_| 0), "Tuesday, March 5, 2024");
        assert_eq!(format_date(ms, "ddd MMM D h:mm A", true, |_| 0), "Tue Mar 5 9:07 AM");
        let pm = (days_from_civil(2024, 3, 5) * DAY + 13 * 3_600_000) as f64;
        assert_eq!(format_date(pm, "h A", true, |_| 0), "1 PM");
    }

    #[test]
    fn to_iso_string_matches_js() {
        assert_eq!(to_iso_string(0.0), "1970-01-01T00:00:00.000Z");
        let ms = (days_from_civil(2024, 3, 5) * DAY + 9 * 3_600_000 + 7 * 60_000 + 3_000 + 45) as f64;
        assert_eq!(to_iso_string(ms), "2024-03-05T09:07:03.045Z");
        assert_eq!(to_iso_string(-DAY as f64), "1969-12-31T00:00:00.000Z");
    }

    #[test]
    fn parse_date_forms() {
        assert_eq!(parse_date("2024-03-05", |_| 0), Some((days_from_civil(2024, 3, 5) * DAY) as f64));
        assert_eq!(parse_date("1970-01-01T00:00:00.000Z", |_| 0), Some(0.0));
        assert_eq!(parse_date("2024-03-05T09:07:03Z", |_| 0), Some(to_ms_utc(2024, 3, 5, 9, 7, 3, 0)));
        assert_eq!(parse_date("2024-03-05T09:07:03.045Z", |_| 0), Some(to_ms_utc(2024, 3, 5, 9, 7, 3, 45)));
        // Explicit offset.
        assert_eq!(parse_date("2024-03-05T09:07:03+05:00", |_| 0), Some(to_ms_utc(2024, 3, 5, 4, 7, 3, 0)));
        assert_eq!(parse_date("2024-03-05T09:07:03-05:00", |_| 0), Some(to_ms_utc(2024, 3, 5, 14, 7, 3, 0)));
        // No Z: local, using the injected fake offset.
        assert_eq!(parse_date("2024-03-05T09:07:03", |_| -300), Some(to_ms_utc(2024, 3, 5, 14, 7, 3, 0)));
        // Invalid.
        assert_eq!(parse_date("not a date", |_| 0), None);
        assert_eq!(parse_date("2024-13-05", |_| 0), None);
        assert_eq!(parse_date("2024-03-05T25:00:00Z", |_| 0), None);
    }

    #[test]
    fn parse_then_format_round_trip() {
        let cases = ["2024-03-05T09:07:03.045Z", "1999-12-31T23:59:59.999Z", "1970-01-01T00:00:00.000Z"];
        for c in cases {
            let ms = parse_date(c, |_| 0).unwrap();
            assert_eq!(to_iso_string(ms), c, "{c}");
        }
    }

    fn to_ms_utc(y: i32, m: i32, d: i32, h: i64, mi: i64, s: i64, ms: i64) -> f64 {
        (days_from_civil(y, m, d) * DAY + h * 3_600_000 + mi * 60_000 + s * 1_000 + ms) as f64
    }
}
