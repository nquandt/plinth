//! Formatting in the locale of the app (`plinth:locale`, core 1.13,
//! `docs/I18N.md` §3.3, §3.7): numbers, dates, plural categories, lists and
//! collation with ICU4X and its compiled data. The web host does the same
//! with the browser's `Intl` (`web/plinth-web.js`, `formatNumber` and the
//! functions after it); keep the two the same. The shared test cases are in
//! `crates/plinth-compiler/tests/locale_cases.txt`.
//!
//! The options come as JSON text from the compiler. An unknown option or
//! value is ignored, and no input makes a call fail: the worst result is
//! an empty string (a value that is not finite) or the number in the
//! "decimal" style (a bad currency code).

use fixed_decimal::{Decimal, FloatPrecision, SignedRoundingMode, UnsignedRoundingMode};
use icu_collator::options::CollatorOptions;
use icu_collator::Collator;
use icu_datetime::fieldsets::builder::{DateFields, FieldSetBuilder};
use icu_datetime::options::{Length, TimePrecision};
use icu_datetime::DateTimeFormatter;
use icu_decimal::options::{DecimalFormatterOptions, GroupingStrategy};
use icu_decimal::{CompactDecimalFormatter, DecimalFormatter};
use icu_experimental::dimension::currency::formatter::CurrencyFormatter;
use icu_experimental::dimension::currency::CurrencyType;
use icu_experimental::dimension::percent::formatter::PercentFormatter;
use icu_list::options::{ListFormatterOptions, ListLength};
use icu_list::ListFormatter;
use icu_locale_core::Locale;
use icu_plurals::{PluralCategory, PluralRules};
use serde_json::Value;
use writeable::Writeable;

/// The ICU4X locale of a BCP 47 tag; `en-US` when ICU4X cannot read it.
fn parse_locale(tag: &str) -> Locale {
    Locale::try_from_str(tag)
        .unwrap_or_else(|_| Locale::try_from_str("en-US").expect("en-US is a valid locale"))
}

fn parse_options(options: &str) -> Value {
    serde_json::from_str(options).unwrap_or(Value::Null)
}

fn opt_str<'a>(o: &'a Value, key: &str) -> Option<&'a str> {
    o.get(key).and_then(Value::as_str)
}

/// A digit count option: an integer from 0 to 20 (the same range as `Intl`).
fn opt_digits(o: &Value, key: &str) -> Option<i16> {
    o.get(key)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite())
        .map(|v| v.trunc().clamp(0.0, 20.0) as i16)
}

/// The shortest decimal that gives `value` back (as JavaScript prints it).
fn to_decimal(value: f64) -> Decimal {
    Decimal::try_from_f64(value, FloatPrecision::RoundTrip).unwrap_or_default()
}

/// Rounds half away from zero (as `Intl`) to `max` fraction digits, then
/// shows at least `min` fraction digits.
fn apply_digits(d: &mut Decimal, min: i16, max: i16) {
    d.round_with_mode(
        -max,
        SignedRoundingMode::Unsigned(UnsignedRoundingMode::HalfExpand),
    );
    d.absolute.trim_end();
    d.absolute.pad_end(-min);
}

/// A valid ISO 4217 code (three ASCII letters), in upper case.
fn currency_code(code: &str) -> Option<CurrencyType> {
    let up = code.to_ascii_uppercase();
    if up.len() != 3 || !up.bytes().all(|b| b.is_ascii_uppercase()) {
        return None;
    }
    CurrencyType::try_from_str(&up).ok()
}

/// `locale.format-number`.
pub fn format_number(tag: &str, value: f64, options: &str) -> String {
    if !value.is_finite() {
        return String::new();
    }
    let loc = parse_locale(tag);
    let o = parse_options(options);
    let style = opt_str(&o, "style").unwrap_or("decimal");
    let grouping = o
        .get("useGrouping")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let decimal_options: DecimalFormatterOptions = if grouping {
        GroupingStrategy::Auto
    } else {
        GroupingStrategy::Never
    }
    .into();
    let min = opt_digits(&o, "minimumFractionDigits");
    let max = opt_digits(&o, "maximumFractionDigits");
    match style {
        "currency" => {
            // The currency sets the fraction digits (ISO 4217 data).
            if let Some(code) = opt_str(&o, "currency").and_then(currency_code) {
                if let Ok(f) =
                    CurrencyFormatter::try_new_symbol((&loc).into(), code, Default::default())
                {
                    return f
                        .format_fixed_decimal(&to_decimal(value))
                        .write_to_string()
                        .into_owned();
                }
            }
        }
        "percent" => {
            let mut d = to_decimal(value);
            d.multiply_pow10(2);
            d.absolute.trim_start();
            let min = min.unwrap_or(0);
            apply_digits(&mut d, min, max.unwrap_or(0).max(min));
            if let Ok(df) = DecimalFormatter::try_new((&loc).into(), decimal_options) {
                if let Ok(f) = PercentFormatter::try_new_with_decimal_formatter(
                    (&loc).into(),
                    df,
                    Default::default(),
                ) {
                    return f.format(&d).write_to_string().into_owned();
                }
            }
        }
        _ => {
            if opt_str(&o, "notation") == Some("compact") {
                if let Ok(f) =
                    CompactDecimalFormatter::try_new_short((&loc).into(), Default::default())
                {
                    return f.format(&to_decimal(value)).write_to_string().into_owned();
                }
            }
        }
    }
    let mut d = to_decimal(value);
    let min = min.unwrap_or(0);
    apply_digits(&mut d, min, max.unwrap_or(3).max(min));
    match DecimalFormatter::try_new((&loc).into(), decimal_options) {
        Ok(f) => f.format(&d).write_to_string().into_owned(),
        Err(_) => d.to_string(),
    }
}

/// Days since 1970-01-01 to (year, month, day) in the proleptic Gregorian
/// calendar (Howard Hinnant's `civil_from_days`).
fn civil(days: i64) -> (i32, u8, u8) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    ((if m <= 2 { y + 1 } else { y }) as i32, m as u8, d as u8)
}

/// `locale.format-date`. `offset_minutes` is the offset of the local time
/// zone at `ms` (minutes east of UTC, as `time.timezone-offset`).
pub fn format_date(tag: &str, ms: f64, offset_minutes: i32, options: &str) -> String {
    // The range of a JavaScript `Date`.
    if !ms.is_finite() || ms.abs() > 8.64e15 {
        return String::new();
    }
    let loc = parse_locale(tag);
    let o = parse_options(options);
    let time_style = opt_str(&o, "timeStyle").filter(|s| matches!(*s, "medium" | "short"));
    let date_style =
        opt_str(&o, "dateStyle").filter(|s| matches!(*s, "full" | "long" | "medium" | "short"));
    let date_style = if date_style.is_none() && time_style.is_none() {
        Some("medium")
    } else {
        date_style
    };
    let local = ms.floor() as i64 + i64::from(offset_minutes) * 60_000;
    let (y, m, d) = civil(local.div_euclid(86_400_000));
    let secs = local.rem_euclid(86_400_000) / 1000;
    let Ok(date) = icu_calendar::Date::try_new_iso(y, m, d) else {
        return String::new();
    };
    let Ok(time) = icu_time::Time::try_new(
        (secs / 3600) as u8,
        ((secs / 60) % 60) as u8,
        (secs % 60) as u8,
        0,
    ) else {
        return String::new();
    };
    let mut b = FieldSetBuilder::default();
    // ICU4X has one length for the date and the time; the date style
    // decides when both are given.
    b.length = Some(match date_style.or(time_style) {
        Some("full" | "long") => Length::Long,
        Some("short") => Length::Short,
        _ => Length::Medium,
    });
    b.date_fields = date_style.map(|s| {
        if s == "full" {
            DateFields::YMDE
        } else {
            DateFields::YMD
        }
    });
    b.time_precision = time_style.map(|s| {
        if s == "short" {
            TimePrecision::Minute
        } else {
            TimePrecision::Second
        }
    });
    let Ok(field_set) = b.build_composite_datetime() else {
        return String::new();
    };
    match DateTimeFormatter::try_new((&loc).into(), field_set) {
        // CLDR puts a narrow no-break space (U+202F) in some patterns, for
        // example before "AM". The browsers give a space (U+0020) there;
        // both hosts give a space (docs/I18N.md §3.7).
        Ok(f) => f
            .format(&icu_time::DateTime { date, time })
            .write_to_string()
            .replace('\u{202f}', " "),
        Err(_) => String::new(),
    }
}

/// `locale.plural-category`: "zero", "one", "two", "few", "many" or "other".
pub fn plural_category(tag: &str, value: f64, kind: &str) -> &'static str {
    if !value.is_finite() {
        return "other";
    }
    let loc = parse_locale(tag);
    let rules = if kind == "ordinal" {
        PluralRules::try_new_ordinal((&loc).into())
    } else {
        PluralRules::try_new_cardinal((&loc).into())
    };
    let Ok(rules) = rules else { return "other" };
    match rules.category_for(&to_decimal(value.abs())) {
        PluralCategory::Zero => "zero",
        PluralCategory::One => "one",
        PluralCategory::Two => "two",
        PluralCategory::Few => "few",
        PluralCategory::Many => "many",
        PluralCategory::Other => "other",
    }
}

/// `locale.format-list`: `items` is a JSON array of strings.
pub fn format_list(tag: &str, items: &str, kind: &str) -> String {
    let loc = parse_locale(tag);
    let items: Vec<String> = serde_json::from_str(items).unwrap_or_default();
    let options = ListFormatterOptions::default().with_length(ListLength::Wide);
    let f = if kind == "disjunction" {
        ListFormatter::try_new_or((&loc).into(), options)
    } else {
        ListFormatter::try_new_and((&loc).into(), options)
    };
    match f {
        Ok(f) => f.format(items.iter()).write_to_string().into_owned(),
        Err(_) => items.join(", "),
    }
}

/// `locale.compare`: -1, 0 or 1.
pub fn compare(tag: &str, a: &str, b: &str) -> i32 {
    let loc = parse_locale(tag);
    let order = match Collator::try_new((&loc).into(), CollatorOptions::default()) {
        Ok(c) => c.compare(a, b),
        Err(_) => a.cmp(b),
    };
    order as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(-1), (1969, 12, 31));
        assert_eq!(civil(20_734), (2026, 10, 8));
        assert_eq!(civil(11_016), (2000, 2, 29));
    }

    #[test]
    fn bad_input_never_fails() {
        assert_eq!(format_number("en-US", f64::NAN, "{}"), "");
        assert_eq!(format_number("en-US", 1.5, "not json"), "1.5");
        assert_eq!(format_number("not a tag!", 1234.0, "{}"), "1,234");
        assert_eq!(
            format_number("en-US", 2.0, r#"{"style":"currency","currency":"E1"}"#),
            "2"
        );
        assert_eq!(
            format_number(
                "en-US",
                2.0,
                r#"{"minimumFractionDigits":99,"maximumFractionDigits":-4}"#
            ),
            "2.00000000000000000000"
        );
        assert_eq!(format_date("en-US", f64::INFINITY, 0, "{}"), "");
        assert_eq!(
            format_date("en-US", 0.0, 0, r#"{"dateStyle":"huge"}"#),
            "Jan 1, 1970"
        );
        assert_eq!(plural_category("en-US", f64::NAN, "cardinal"), "other");
        assert_eq!(format_list("en-US", "[1, 2]", "conjunction"), "");
        assert_eq!(compare("en-US", "a", "b"), -1);
    }

    #[test]
    fn time_zone_offset() {
        // 2026-10-08 11:44:05 UTC at +02:00 is 13:44.
        assert_eq!(
            format_date(
                "de-DE",
                1_791_459_845_000.0,
                120,
                r#"{"timeStyle":"short"}"#
            ),
            "13:44"
        );
        assert_eq!(
            format_date(
                "en-US",
                1_791_459_845_000.0,
                -720,
                r#"{"dateStyle":"short"}"#
            ),
            "10/7/26"
        );
    }
}
