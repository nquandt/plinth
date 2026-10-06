//! Immutable UTF-8 strings on the GC heap (SPEC.md §4.2, §4.6).
//!
//! `length`, indexing and `slice` use UTF-16 code units, as in TS (open
//! question Q5). ASCII strings take a fast path.

use crate::gc::{self, T_STRING, load_u32};

pub fn new_uninit(len: u32) -> u32 {
    let p = gc::alloc(T_STRING, 12 + len);
    unsafe { gc::store_u32(p + 8, len) };
    p
}

pub fn from_str(s: &str) -> u32 {
    let p = new_uninit(s.len() as u32);
    unsafe { std::ptr::copy_nonoverlapping(s.as_ptr(), (p + 12) as *mut u8, s.len()) };
    p
}

/// Borrows the bytes of a string object. A null reference reads as "".
pub fn as_str<'a>(p: u32) -> &'a str {
    if p == 0 {
        return "";
    }
    unsafe {
        let len = load_u32(p + 8) as usize;
        let bytes = std::slice::from_raw_parts((p + 12) as *const u8, len);
        std::str::from_utf8_unchecked(bytes)
    }
}

pub fn len16(s: &str) -> usize {
    if s.is_ascii() { s.len() } else { s.encode_utf16().count() }
}

/// Converts a UTF-16 index range to a byte range.
fn byte_range(s: &str, start: usize, end: usize) -> (usize, usize) {
    if s.is_ascii() {
        return (start.min(s.len()), end.min(s.len()));
    }
    let mut units = 0;
    let (mut b_start, mut b_end) = (s.len(), s.len());
    for (i, c) in s.char_indices() {
        if units >= start && b_start == s.len() {
            b_start = i;
        }
        if units >= end {
            b_end = i;
            break;
        }
        units += c.len_utf16();
    }
    (b_start.min(b_end), b_end)
}

/// Resolves a JS-style relative index (negative counts from the end).
fn rel_index(i: f64, len: usize) -> usize {
    if i.is_nan() {
        return 0;
    }
    let i = i.trunc();
    if i < 0.0 { (len as f64 + i).max(0.0) as usize } else { i.min(len as f64) as usize }
}

pub fn slice(p: u32, start: f64, end: f64) -> u32 {
    let s = as_str(p);
    let len = len16(s);
    let a = rel_index(start, len);
    let b = if end.is_nan() { len } else { rel_index(end, len) };
    if a >= b {
        return from_str("");
    }
    let (x, y) = byte_range(s, a, b);
    from_str(&s[x..y])
}

pub fn index_of(p: u32, q: u32) -> f64 {
    let (s, q) = (as_str(p), as_str(q));
    match s.find(q) {
        Some(i) => len16(&s[..i]) as f64,
        None => -1.0,
    }
}

/// JS `Number.prototype.toString()` for base 10.
pub fn number_to_string(v: f64) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v == 0.0 {
        return "0".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if v < 0.0 {
        return format!("-{}", number_to_string(-v));
    }
    // Rust gives the shortest round-trip digits; JS uses the same digits.
    let sci = format!("{v:e}");
    let (mantissa, exp) = sci.split_once('e').unwrap();
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i32;
    let n = exp.parse::<i32>().unwrap() + 1;
    if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let e = n - 1;
        let sign = if e >= 0 { "+" } else { "-" };
        if k == 1 {
            format!("{digits}e{sign}{}", e.abs())
        } else {
            format!("{}.{}e{sign}{}", &digits[..1], &digits[1..], e.abs())
        }
    }
}

/// JS `toFixed` for 0..=20 digits.
pub fn to_fixed(v: f64, digits: f64) -> String {
    let d = digits.clamp(0.0, 20.0) as usize;
    if !v.is_finite() || v.abs() >= 1e21 {
        return number_to_string(v);
    }
    format!("{v:.d$}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_number_format() {
        let cases = [
            (1.0, "1"),
            (-2.0, "-2"),
            (0.5, "0.5"),
            (123.456, "123.456"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (1e-7, "1e-7"),
            (0.000001, "0.000001"),
            (1.5e-10, "1.5e-10"),
            (0.1 + 0.2, "0.30000000000000004"),
            (f64::NAN, "NaN"),
            (f64::INFINITY, "Infinity"),
            (-0.0, "0"),
        ];
        for (v, s) in cases {
            assert_eq!(number_to_string(v), s, "{v}");
        }
    }

    #[test]
    fn utf16_ranges() {
        let s = "aé😀b";
        assert_eq!(len16(s), 5);
        assert_eq!(byte_range(s, 2, 4), (3, 7));
        assert_eq!(rel_index(-1.0, 5), 4);
    }
}
