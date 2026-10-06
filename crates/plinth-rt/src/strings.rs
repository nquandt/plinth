//! Immutable UTF-8 strings on the GC heap (SPEC.md §4.2, §4.6).
//!
//! `length`, indexing and `slice` use UTF-16 code units, as in TS (open
//! question Q5). ASCII strings take a fast path.
//!
//! This module does not use `core::fmt`, which is large: numbers format
//! through `ryu` and the code below.

use crate::gc::{self, T_STRING, load_u32};
use alloc::string::String;
use alloc::vec::Vec;

pub fn new_uninit(len: u32) -> u32 {
    let p = gc::alloc(T_STRING, 12 + len);
    unsafe { gc::store_u32(p + 8, len) };
    p
}

pub fn from_str(s: &str) -> u32 {
    let p = new_uninit(s.len() as u32);
    unsafe { core::ptr::copy_nonoverlapping(s.as_ptr(), (p + 12) as *mut u8, s.len()) };
    p
}

/// Borrows the bytes of a string object. A null reference reads as "".
pub fn as_str<'a>(p: u32) -> &'a str {
    if p == 0 {
        return "";
    }
    unsafe {
        let len = load_u32(p + 8) as usize;
        let bytes = core::slice::from_raw_parts((p + 12) as *const u8, len);
        core::str::from_utf8_unchecked(bytes)
    }
}

pub fn len16(s: &str) -> usize {
    if s.is_ascii() { s.len() } else { s.encode_utf16().count() }
}

/// `JSON.stringify` of a number (SPEC.md §4.7): `NaN`/`Infinity` become the
/// `null` token, like JS; everything else formats like `toString`.
pub fn json_number_to_string(v: f64) -> String {
    if v.is_finite() { number_to_string(v) } else { String::from("null") }
}

const HEX: &[u8; 16] = b"0123456789abcdef";

/// `JSON.stringify` of a string: a quoted, escaped JSON string literal.
/// Non-ASCII bytes pass through unescaped (valid UTF-8 JSON).
pub fn json_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for b in s.bytes() {
        match b {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            0x08 => out.push_str("\\b"),
            0x0c => out.push_str("\\f"),
            0x00..=0x1f => {
                out.push_str("\\u00");
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 0xf) as usize] as char);
            }
            _ => unsafe { out.as_mut_vec().push(b) },
        }
    }
    out.push('"');
    out
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
    let i = libm::trunc(i);
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

fn push_u64(out: &mut String, mut v: u64) {
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    out.push_str(core::str::from_utf8(&buf[i..]).unwrap());
}

/// The shortest round-trip digits of a finite positive number, and the
/// position of the decimal point (`n` in the ECMAScript algorithm).
fn shortest(v: f64) -> (Vec<u8>, i32) {
    let mut buf = ryu::Buffer::new();
    let s = buf.format_finite(v).as_bytes();
    // ryu writes "123.45", "0.001" or "1.5e-10".
    let (mantissa, exp) = match s.iter().position(|&c| c == b'e') {
        Some(i) => {
            let e = &s[i + 1..];
            let (neg, digits) = if e[0] == b'-' { (true, &e[1..]) } else { (false, e) };
            let mut x = 0i32;
            for &d in digits {
                x = x * 10 + (d - b'0') as i32;
            }
            (&s[..i], if neg { -x } else { x })
        }
        None => (s, 0),
    };
    let point = mantissa.iter().position(|&c| c == b'.').unwrap_or(mantissa.len());
    let mut digits: Vec<u8> = mantissa.iter().copied().filter(|&c| c != b'.').collect();
    let mut n = point as i32 + exp;
    while digits.first() == Some(&b'0') && digits.len() > 1 {
        digits.remove(0);
        n -= 1;
    }
    while digits.last() == Some(&b'0') && digits.len() > 1 {
        digits.pop();
    }
    (digits, n)
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
    let mut out = String::new();
    if v < 0.0 {
        out.push('-');
    }
    let (digits, n) = shortest(v.abs());
    let k = digits.len() as i32;
    let d = |a: usize, b: usize| core::str::from_utf8(&digits[a..b]).unwrap();
    if k <= n && n <= 21 {
        out.push_str(d(0, k as usize));
        for _ in 0..(n - k) {
            out.push('0');
        }
    } else if 0 < n && n <= 21 {
        out.push_str(d(0, n as usize));
        out.push('.');
        out.push_str(d(n as usize, k as usize));
    } else if -6 < n && n <= 0 {
        out.push_str("0.");
        for _ in 0..(-n) {
            out.push('0');
        }
        out.push_str(d(0, k as usize));
    } else {
        let e = n - 1;
        out.push_str(d(0, 1));
        if k > 1 {
            out.push('.');
            out.push_str(d(1, k as usize));
        }
        out.push('e');
        out.push(if e >= 0 { '+' } else { '-' });
        push_u64(&mut out, e.unsigned_abs() as u64);
    }
    out
}

/// JS `toFixed` for 0..=20 digits, with exact rounding of the binary value
/// (half up), as the ECMAScript algorithm specifies. The fraction digits
/// come from binary long division, so no 128-bit division is needed.
pub fn to_fixed(v: f64, digits: f64) -> String {
    let d = if digits.is_nan() { 0 } else { digits.clamp(0.0, 20.0) as usize };
    if !v.is_finite() || v.abs() >= 1e21 {
        return number_to_string(v);
    }
    let bits = v.to_bits();
    let neg = bits >> 63 == 1 && v != 0.0;
    let exp = ((bits >> 52) & 0x7ff) as i32;
    let frac = bits & ((1u64 << 52) - 1);
    let (m, e) = if exp == 0 { (frac, -1074) } else { (frac | (1u64 << 52), exp - 1075) };
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    if e >= 0 {
        // An integer. Above 2^64 the digits come from `toString` (a small
        // deviation: JS prints the exact binary value there).
        match (m as u128).checked_shl(e as u32).filter(|x| *x <= u64::MAX as u128) {
            Some(x) => push_u64(&mut out, x as u64),
            None => out.push_str(&number_to_string(v.abs())),
        }
        if d > 0 {
            out.push('.');
            for _ in 0..d {
                out.push('0');
            }
        }
        return out;
    }
    let k = (-e) as u32;
    let mut int = if k >= 64 { 0 } else { m >> k };
    // The fraction is r / 2^k.
    let mut digs = [0u8; 20];
    let mut round_up = false;
    if k <= 124 {
        let mask = (1u128 << k) - 1;
        let mut r = if k >= 64 { m as u128 } else { (m as u128) & mask };
        for slot in digs.iter_mut().take(d) {
            r *= 10;
            *slot = (r >> k) as u8;
            r &= mask;
        }
        round_up = r << 1 >= 1u128 << k;
    }
    if round_up {
        let mut i = d;
        loop {
            if i == 0 {
                int += 1;
                break;
            }
            i -= 1;
            if digs[i] == 9 {
                digs[i] = 0;
            } else {
                digs[i] += 1;
                break;
            }
        }
    }
    push_u64(&mut out, int);
    if d > 0 {
        out.push('.');
        for &x in &digs[..d] {
            out.push((b'0' + x) as char);
        }
    }
    out
}

const POW10: [f64; 23] = [
    1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16, 1e17, 1e18, 1e19, 1e20,
    1e21, 1e22,
];

/// JS `Number(s)` for decimal strings. The result is exact when the value
/// has at most 15 significant digits and a small exponent (the common
/// case); otherwise it can differ by one unit in the last place.
pub fn parse_number(s: &str) -> f64 {
    let s = s.trim();
    if s.is_empty() {
        return 0.0;
    }
    let (neg, body) = match s.as_bytes()[0] {
        b'-' => (true, &s[1..]),
        b'+' => (false, &s[1..]),
        _ => (false, s),
    };
    let sign = if neg { -1.0 } else { 1.0 };
    if body == "Infinity" {
        return sign * f64::INFINITY;
    }
    let b = body.as_bytes();
    let (mut mant, mut exp10, mut digits, mut seen) = (0u64, 0i32, 0u32, false);
    let mut i = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        seen = true;
        if digits < 19 {
            mant = mant * 10 + (b[i] - b'0') as u64;
            if mant != 0 {
                digits += 1;
            }
        } else {
            exp10 += 1;
        }
        i += 1;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            seen = true;
            if digits < 19 {
                mant = mant * 10 + (b[i] - b'0') as u64;
                exp10 -= 1;
                if mant != 0 {
                    digits += 1;
                }
            }
            i += 1;
        }
    }
    if !seen {
        return f64::NAN;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        let eneg = i < b.len() && b[i] == b'-';
        if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
            i += 1;
        }
        let start = i;
        let mut e = 0i32;
        while i < b.len() && b[i].is_ascii_digit() {
            e = e.saturating_mul(10).saturating_add((b[i] - b'0') as i32);
            i += 1;
        }
        if i == start {
            return f64::NAN;
        }
        exp10 = exp10.saturating_add(if eneg { -e } else { e });
    }
    if i != b.len() {
        return f64::NAN;
    }
    if mant == 0 {
        return sign * 0.0;
    }
    let m = mant as f64;
    let v = if mant < (1u64 << 53) && (-22..=22).contains(&exp10) {
        if exp10 >= 0 { m * POW10[exp10 as usize] } else { m / POW10[(-exp10) as usize] }
    } else {
        m * libm::pow(10.0, exp10 as f64)
    };
    sign * v
}

// -- Case mapping --------------------------------------------------------------
//
// ASCII, Latin-1, Latin Extended-A, Greek and Cyrillic. Other characters do
// not change (a documented deviation; the full Unicode tables are large).

fn upper(c: char) -> char {
    let u = c as u32;
    let m = match u {
        0x61..=0x7A => u - 32,
        0xB5 => 0x39C,
        0xE0..=0xFE if u != 0xF7 => u - 32,
        0xFF => 0x178,
        0x100..=0x137 | 0x14A..=0x177 if u % 2 == 1 => u - 1,
        0x139..=0x148 | 0x179..=0x17E if u % 2 == 0 => u - 1,
        0x131 => 0x49,
        0x17F => 0x53,
        0x3AC => 0x386,
        0x3AD..=0x3AF => u - 37,
        0x3B1..=0x3C1 | 0x3C3..=0x3CB => u - 32,
        0x3C2 => 0x3A3,
        0x3CC => 0x38C,
        0x3CD..=0x3CE => u - 63,
        0x430..=0x44F => u - 32,
        0x450..=0x45F => u - 80,
        0x460..=0x481 | 0x48A..=0x4BF if u % 2 == 1 => u - 1,
        _ => u,
    };
    char::from_u32(m).unwrap_or(c)
}

fn lower(c: char) -> char {
    let u = c as u32;
    let m = match u {
        0x41..=0x5A => u + 32,
        0xC0..=0xDE if u != 0xD7 => u + 32,
        0x178 => 0xFF,
        0x100..=0x137 | 0x14A..=0x177 if u % 2 == 0 => u + 1,
        0x139..=0x148 | 0x179..=0x17E if u % 2 == 1 => u + 1,
        0x130 => 0x69,
        0x386 => 0x3AC,
        0x388..=0x38A => u + 37,
        0x38C => 0x3CC,
        0x38E..=0x38F => u + 63,
        0x391..=0x3A1 | 0x3A3..=0x3AB => u + 32,
        0x410..=0x42F => u + 32,
        0x400..=0x40F => u + 80,
        0x460..=0x481 | 0x48A..=0x4BF if u % 2 == 0 => u + 1,
        _ => u,
    };
    char::from_u32(m).unwrap_or(c)
}

pub fn to_upper(s: &str) -> String {
    if s.is_ascii() {
        return s.to_ascii_uppercase();
    }
    s.chars().map(upper).collect()
}

pub fn to_lower(s: &str) -> String {
    if s.is_ascii() {
        return s.to_ascii_lowercase();
    }
    s.chars().map(lower).collect()
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
            (100.0, "100"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (1e-7, "1e-7"),
            (0.000001, "0.000001"),
            (1.5e-10, "1.5e-10"),
            (0.1 + 0.2, "0.30000000000000004"),
            (123e25, "1.23e+27"),
            (f64::NAN, "NaN"),
            (f64::INFINITY, "Infinity"),
            (-0.0, "0"),
            (5e-324, "5e-324"),
            (1.7976931348623157e308, "1.7976931348623157e+308"),
        ];
        for (v, s) in cases {
            assert_eq!(number_to_string(v), s, "{v}");
        }
    }

    #[test]
    fn js_to_fixed() {
        let cases = [
            (1.005, 2.0, "1.00"),
            (1.5, 0.0, "2"),
            (2.5, 0.0, "3"),
            (0.125, 2.0, "0.13"),
            (123.456, 1.0, "123.5"),
            (-1.25, 1.0, "-1.3"),
            (0.0, 2.0, "0.00"),
            (10.0, 3.0, "10.000"),
            (0.000001, 2.0, "0.00"),
            (1e21, 2.0, "1e+21"),
            (99.995, 2.0, "100.00"),
        ];
        for (v, d, s) in cases {
            assert_eq!(to_fixed(v, d), s, "{v}.toFixed({d})");
        }
    }

    #[test]
    fn parse_numbers() {
        let cases = [
            ("42", 42.0),
            (" -3.5 ", -3.5),
            ("1e3", 1000.0),
            ("0.1", 0.1),
            (".5", 0.5),
            ("", 0.0),
            ("+Infinity", f64::INFINITY),
            ("123456789012345678", 123456789012345680.0),
        ];
        for (s, v) in cases {
            assert_eq!(parse_number(s), v, "{s}");
        }
        assert!(parse_number("abc").is_nan());
        assert!(parse_number("1e").is_nan());
        assert!(parse_number("1.2.3").is_nan());
    }

    #[test]
    fn case_mapping() {
        assert_eq!(to_upper("straße àé ωσς привет"), "STRAßE ÀÉ ΩΣΣ ПРИВЕТ");
        assert_eq!(to_lower("ÀÉÎ ΑΒΓ ПРИВЕТ Ÿ"), "àéî αβγ привет ÿ");
    }

    #[test]
    fn utf16_ranges() {
        let s = "aé😀b";
        assert_eq!(len16(s), 5);
        assert_eq!(byte_range(s, 2, 4), (3, 7));
        assert_eq!(rel_index(-1.0, 5), 4);
    }
}
