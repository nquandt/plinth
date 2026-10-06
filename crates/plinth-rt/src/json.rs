//! `JSON.parse<T>` (SPEC.md §4.7): a small cursor-based tokenizer. The
//! compiler generates a typed decoder per `T` (mirrors `json_stringify_value`
//! in `check/expr.rs`); this module only tokenizes and tracks one "did this
//! parse fail" flag, so a decoder can keep calling these functions after a
//! failure (they return safe defaults) and check `ok()` once at the end.
//!
//! Parsing is not reentrant: a decoder runs to completion before the app
//! code can call `JSON.parse` again, so plain globals are enough (the guest
//! is single-threaded, SPEC.md §4.5).

use crate::global::GlobalCell;
use crate::strings;
use alloc::string::String;
#[cfg(test)]
use alloc::vec::Vec;

static TEXT: GlobalCell<u32> = GlobalCell::new(0);
static POS: GlobalCell<u32> = GlobalCell::new(0);
static OK: GlobalCell<bool> = GlobalCell::new(false);

pub const KIND_NULL: i32 = 0;
pub const KIND_BOOL: i32 = 1;
pub const KIND_NUM: i32 = 2;
pub const KIND_STR: i32 = 3;
pub const KIND_ARR: i32 = 4;
pub const KIND_OBJ: i32 = 5;
pub const KIND_INVALID: i32 = 6;

fn text() -> &'static str {
    strings::as_str(TEXT.get())
}

fn fail() {
    OK.set(false);
}

fn rest(s: &str) -> &[u8] {
    let b = s.as_bytes();
    &b[(POS.get() as usize).min(b.len())..]
}

fn skip_ws() {
    let s = text();
    let b = s.as_bytes();
    let mut i = POS.get() as usize;
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
        i += 1;
    }
    POS.set(i as u32);
}

fn peek() -> Option<u8> {
    rest(text()).first().copied()
}

fn advance(n: usize) {
    POS.set(POS.get() + n as u32);
}

/// Consumes a literal keyword (`"true"`, `"false"`, `"null"`) if present.
fn eat_literal(lit: &str) -> bool {
    if rest(text()).starts_with(lit.as_bytes()) {
        advance(lit.len());
        true
    } else {
        false
    }
}

pub fn begin(text_ptr: u32) {
    TEXT.set(text_ptr);
    POS.set(0);
    OK.set(true);
    skip_ws();
}

pub fn ok() -> i32 {
    OK.get() as i32
}

/// Checks that only whitespace is left, and combines that with the overall
/// `ok` flag. Call once, after the top-level value is read.
pub fn finish() -> i32 {
    skip_ws();
    if POS.get() as usize != text().len() {
        fail();
    }
    OK.get() as i32
}

pub fn fail_now() {
    fail();
}

/// The kind of the value at the cursor, without consuming it.
pub fn peek_kind() -> i32 {
    if !OK.get() {
        return KIND_INVALID;
    }
    skip_ws();
    match peek() {
        Some(b'n') => KIND_NULL,
        Some(b't') | Some(b'f') => KIND_BOOL,
        Some(b'"') => KIND_STR,
        Some(b'[') => KIND_ARR,
        Some(b'{') => KIND_OBJ,
        Some(b'-') | Some(b'0'..=b'9') => KIND_NUM,
        _ => KIND_INVALID,
    }
}

pub fn read_null() {
    if !OK.get() {
        return;
    }
    skip_ws();
    if !eat_literal("null") {
        fail();
    }
}

pub fn read_bool() -> i32 {
    if !OK.get() {
        return 0;
    }
    skip_ws();
    if eat_literal("true") {
        1
    } else if eat_literal("false") {
        0
    } else {
        fail();
        0
    }
}

/// The byte length of a JSON number token starting at the cursor, or `None`
/// if there is not a valid one.
fn number_span(b: &[u8]) -> Option<usize> {
    let mut i = 0;
    if i < b.len() && b[i] == b'-' {
        i += 1;
    }
    let int_start = i;
    if i < b.len() && b[i] == b'0' {
        i += 1;
    } else {
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i == int_start {
        return None;
    }
    if i < b.len() && b[i] == b'.' {
        let dot = i;
        i += 1;
        let frac_start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == frac_start {
            i = dot;
        }
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let e = i;
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        let exp_start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == exp_start {
            i = e;
        }
    }
    Some(i)
}

pub fn read_num() -> f64 {
    if !OK.get() {
        return 0.0;
    }
    skip_ws();
    let s = text();
    let b = rest(s);
    match number_span(b) {
        Some(n) if n > 0 => {
            let start = POS.get() as usize;
            advance(n);
            strings::parse_number(&s[start..start + n])
        }
        _ => {
            fail();
            0.0
        }
    }
}

pub fn read_int() -> i32 {
    let v = read_num();
    if !OK.get() {
        return 0;
    }
    let t = libm::trunc(v);
    if t != v || t < i32::MIN as f64 || t > i32::MAX as f64 {
        fail();
        return 0;
    }
    t as i32
}

/// Decodes a quoted JSON string at the cursor into a new heap string.
/// Handles escapes, including `\uXXXX` and surrogate pairs.
pub fn read_str() -> u32 {
    if !OK.get() {
        return 0;
    }
    skip_ws();
    let s = text();
    let b = s.as_bytes();
    let mut i = POS.get() as usize;
    if i >= b.len() || b[i] != b'"' {
        fail();
        return 0;
    }
    i += 1;
    let mut out = String::new();
    loop {
        if i >= b.len() {
            fail();
            return 0;
        }
        match b[i] {
            b'"' => {
                i += 1;
                break;
            }
            b'\\' => {
                i += 1;
                if i >= b.len() {
                    fail();
                    return 0;
                }
                match b[i] {
                    b'"' => {
                        out.push('"');
                        i += 1;
                    }
                    b'\\' => {
                        out.push('\\');
                        i += 1;
                    }
                    b'/' => {
                        out.push('/');
                        i += 1;
                    }
                    b'b' => {
                        out.push('\u{0008}');
                        i += 1;
                    }
                    b'f' => {
                        out.push('\u{000c}');
                        i += 1;
                    }
                    b'n' => {
                        out.push('\n');
                        i += 1;
                    }
                    b'r' => {
                        out.push('\r');
                        i += 1;
                    }
                    b't' => {
                        out.push('\t');
                        i += 1;
                    }
                    b'u' => {
                        i += 1;
                        let Some(u1) = hex4(b, i) else {
                            fail();
                            return 0;
                        };
                        i += 4;
                        let cp = if (0xD800..=0xDBFF).contains(&u1) {
                            if i + 1 < b.len() && b[i] == b'\\' && b[i + 1] == b'u' {
                                let Some(u2) = hex4(b, i + 2) else {
                                    fail();
                                    return 0;
                                };
                                if (0xDC00..=0xDFFF).contains(&u2) {
                                    i += 6;
                                    0x10000 + ((u1 - 0xD800) as u32) * 0x400 + (u2 - 0xDC00) as u32
                                } else {
                                    fail();
                                    return 0;
                                }
                            } else {
                                fail();
                                return 0;
                            }
                        } else {
                            u1 as u32
                        };
                        match char::from_u32(cp) {
                            Some(c) => out.push(c),
                            None => {
                                fail();
                                return 0;
                            }
                        }
                    }
                    _ => {
                        fail();
                        return 0;
                    }
                }
            }
            0x00..=0x1f => {
                fail();
                return 0;
            }
            _ => {
                // Copy a whole UTF-8 sequence (the source is valid UTF-8).
                let start = i;
                i += 1;
                while i < b.len() && (b[i] & 0b1100_0000) == 0b1000_0000 {
                    i += 1;
                }
                out.push_str(&s[start..i]);
            }
        }
    }
    POS.set(i as u32);
    strings::from_str(&out)
}

fn hex4(b: &[u8], i: usize) -> Option<u16> {
    if i + 4 > b.len() {
        return None;
    }
    let mut v: u16 = 0;
    for &c in &b[i..i + 4] {
        let d = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => return None,
        };
        v = v * 16 + d as u16;
    }
    Some(v)
}

/// Skips the value at the cursor, whatever kind it is (used for an object
/// key the decoder's type does not have).
pub fn skip_value() {
    if !OK.get() {
        return;
    }
    skip_ws();
    match peek() {
        Some(b'"') => {
            read_str();
        }
        Some(b'[') => {
            arr_begin();
            while arr_next() != 0 {
                skip_value();
            }
        }
        Some(b'{') => {
            obj_begin();
            loop {
                let k = obj_next_key();
                if k == 0 {
                    break;
                }
                skip_value();
            }
        }
        Some(b't') | Some(b'f') => {
            read_bool();
        }
        Some(b'n') => read_null(),
        Some(b'-') | Some(b'0'..=b'9') => {
            read_num();
        }
        _ => fail(),
    }
}

pub fn arr_begin() {
    if !OK.get() {
        return;
    }
    skip_ws();
    if peek() == Some(b'[') {
        advance(1);
        skip_ws();
    } else {
        fail();
    }
}

/// `1` if another array element follows (the cursor is positioned at it),
/// `0` if the array is done (or on failure).
pub fn arr_next() -> i32 {
    if !OK.get() {
        return 0;
    }
    skip_ws();
    match peek() {
        Some(b']') => {
            advance(1);
            0
        }
        None => {
            fail();
            0
        }
        Some(b',') => {
            advance(1);
            skip_ws();
            if peek() == Some(b']') {
                fail();
                0
            } else {
                1
            }
        }
        _ => 1,
    }
}

static FIRST_KEY: GlobalCell<bool> = GlobalCell::new(true);

pub fn obj_begin() {
    if !OK.get() {
        return;
    }
    skip_ws();
    if peek() == Some(b'{') {
        advance(1);
        skip_ws();
        FIRST_KEY.set(true);
    } else {
        fail();
    }
}

/// The next key as a heap string, or `0` (null) when the object is done (or
/// on failure). Positions the cursor at the value, after the `:`.
pub fn obj_next_key() -> i32 {
    if !OK.get() {
        return 0;
    }
    skip_ws();
    let first = FIRST_KEY.get();
    FIRST_KEY.set(false);
    match peek() {
        Some(b'}') => {
            advance(1);
            0
        }
        Some(b',') if !first => {
            advance(1);
            skip_ws();
            read_key()
        }
        _ if first => read_key(),
        _ => {
            fail();
            0
        }
    }
}

// The functions above read from a GC string through `plinth-rt`'s Wasm
// linear memory, so they only run inside a real guest instance; the
// compiler's e2e tests (`lang.rs`, the "JSON" section) exercise them end to
// end. These are plain, memory-free unit tests for the token-boundary
// helpers.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_spans() {
        assert_eq!(number_span(b"42"), Some(2));
        assert_eq!(number_span(b"-42.5"), Some(5));
        assert_eq!(number_span(b"1e10,"), Some(4));
        assert_eq!(number_span(b"1.5e-3]"), Some(6));
        assert_eq!(number_span(b"-"), None);
        assert_eq!(number_span(b"abc"), None);
        assert_eq!(number_span(b"0.5"), Some(3));
    }

    #[test]
    fn hex_escapes() {
        assert_eq!(hex4(b"0041", 0), Some(0x41));
        assert_eq!(hex4(b"d83d", 0), Some(0xd83d));
        assert_eq!(hex4(b"zzzz", 0), None);
        assert_eq!(hex4(b"12", 0), None);
    }
}

fn read_key() -> i32 {
    skip_ws();
    if peek() != Some(b'"') {
        fail();
        return 0;
    }
    let key = read_str();
    if !OK.get() {
        return 0;
    }
    skip_ws();
    if peek() == Some(b':') {
        advance(1);
        skip_ws();
        key as i32
    } else {
        fail();
        0
    }
}
