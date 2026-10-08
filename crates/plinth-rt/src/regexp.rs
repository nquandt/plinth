//! The `RegExp` ABI (core 1.12, PRODUCTION.md L4 step 2): glues the pure
//! Pike-VM matcher in `regex.rs` to GC strings and to JS's UTF-16-unit
//! indices (`strings.rs` uses those for `length`/`slice`/etc; `regex.rs`
//! itself works in `char` indices — see its module docs).
//!
//! A compiled pattern is **not** itself traced by the GC (the `Program`'s
//! NFA instructions are a plain Rust `Vec`, not GC memory): it lives in a
//! host-side table here, addressed by a small `i32` handle, the same shape
//! as `ui.rs`'s event-handler table (a `Vec<Option<Compiled>>` plus a
//! free-list for handle reuse). But the **value a `RegExp` variable holds
//! at run time is not that raw handle** — it is a tiny GC object (type id
//! `gc::T_REGEXP`) that only wraps the handle. That box is what makes a
//! `RegExp` collectable: `gc::collect`'s sweep calls `release` below for
//! every `T_REGEXP` box it frees, which drops the matching table entry (and
//! with it the `Program`, `pattern` and `flags` strings). Without this, a
//! regex literal re-evaluated on every render or event call (`onPress={()
//! => /x/.test(s)}`) would leak one table entry per evaluation forever
//! (HANDOFF.md §6: app memory must stay level). `.source`/`.flags` are
//! rebuilt into fresh GC strings on demand from the `Compiled` entry's own
//! (non-GC) `String`s, so nothing needs separate rooting for those.

use crate::global::{Global, GlobalCell};
use crate::{arrays, gc, strings};
use alloc::string::String;
use alloc::vec::Vec;

struct Compiled {
    prog: crate::regex::Program,
    pattern: String,
    flags: String,
    global: bool,
    /// UTF-16 code units, like JS's `lastIndex`.
    last_index: f64,
}

static TABLE: Global<Vec<Option<Compiled>>> = Global::new(Vec::new());
static FREE: Global<Vec<u32>> = Global::new(Vec::new());
static LAST_ERROR: Global<String> = Global::new(String::new());
static LAST_MATCH_INDEX: GlobalCell<f64> = GlobalCell::new(0.0);

fn with_compiled<R>(handle: i32, f: impl FnOnce(&Compiled) -> R) -> R {
    TABLE.read(|t| f(t[(handle as usize) - 1].as_ref().expect("a live RegExp handle")))
}

fn with_compiled_mut<R>(handle: i32, f: impl FnOnce(&mut Compiled) -> R) -> R {
    TABLE.with(|t| f(t[(handle as usize) - 1].as_mut().expect("a live RegExp handle")))
}

/// Wraps a freshly compiled handle (or `0`, meaning "no pattern") in a
/// `T_REGEXP` GC box: the value the app actually sees and stores. Counts
/// toward `allocated_since_gc` like any other allocation (`gc::alloc`
/// already does this), so a loop that makes many of these still triggers
/// a collection instead of growing memory unboundedly.
fn box_handle(handle: i32) -> i32 {
    if handle == 0 {
        return 0;
    }
    let p = gc::alloc(gc::T_REGEXP, 16);
    unsafe { gc::store_u32(p + 8, handle as u32) };
    p as i32
}

/// Unwraps a `T_REGEXP` GC box back to its table handle.
fn unbox(boxed: i32) -> i32 {
    if boxed == 0 {
        crate::trap("null RegExp used");
    }
    unsafe { gc::load_u32(boxed as u32 + 8) as i32 }
}

/// Called by `gc::collect`'s sweep when a `T_REGEXP` box is freed: the
/// `RegExp` it wrapped is unreachable, so its table entry (and the
/// `Program` it owns) can go too.
pub fn release(handle: i32) {
    if handle == 0 {
        return;
    }
    let idx = (handle as usize) - 1;
    TABLE.with(|t| {
        if idx < t.len() {
            t[idx] = None;
        }
    });
    FREE.with(|f| f.push(handle as u32));
}

/// Converts a `char`-index position in `s` to a UTF-16-unit index (as in
/// JS). ASCII strings take the fast path used throughout `strings.rs`.
fn char_to_utf16(s: &str, char_idx: usize) -> usize {
    if s.is_ascii() {
        return char_idx;
    }
    s.chars().take(char_idx).map(char::len_utf16).sum()
}

/// Converts a UTF-16-unit position to a `char` index, clamped to the
/// string's length. The inverse of `char_to_utf16`.
fn utf16_to_char(s: &str, u16_idx: usize) -> usize {
    if s.is_ascii() {
        return u16_idx.min(s.len());
    }
    let mut units = 0;
    for (i, c) in s.chars().enumerate() {
        if units >= u16_idx {
            return i;
        }
        units += c.len_utf16();
    }
    s.chars().count()
}

fn flags_from_str(s: &str) -> Option<crate::regex::Flags> {
    let mut flags = crate::regex::Flags::default();
    for c in s.chars() {
        match c {
            'g' => flags.global = true,
            'i' => flags.ignore_case = true,
            'm' => flags.multiline = true,
            's' => flags.dot_all = true,
            _ => return None,
        }
    }
    Some(flags)
}

fn error_message(e: crate::regex::ErrorKind) -> &'static str {
    use crate::regex::ErrorKind;
    match e {
        ErrorKind::UnterminatedGroup => "unterminated group",
        ErrorKind::UnterminatedClass => "unterminated character class",
        ErrorKind::InvalidQuantifier => "invalid quantifier",
        ErrorKind::QuantifierWithoutTarget => "quantifier with nothing to repeat",
        ErrorKind::TrailingEscape => "trailing backslash",
        ErrorKind::UnmatchedCloseParen => "unmatched `)`",
        ErrorKind::Unsupported => "lookaround and backreferences are not supported",
    }
}

/// `new RegExp(pattern, flags)` / a `/pattern/flags` literal: compiles and
/// returns a boxed (GC-collectable) `RegExp` value, or `0` on a bad
/// pattern/flag (with the message left in `regex_last_error()` for the
/// compiler-generated `throw` to read).
pub fn compile(pattern: &str, flags_str: &str) -> i32 {
    let flags = match flags_from_str(flags_str) {
        Some(f) => f,
        None => {
            let bad = flags_str.chars().find(|c| !matches!(c, 'g' | 'i' | 'm' | 's')).unwrap_or('?');
            LAST_ERROR.with(|e| {
                e.clear();
                e.push_str("invalid regular expression flag \"");
                e.push(bad);
                e.push('"');
            });
            return 0;
        }
    };
    match crate::regex::parse(pattern, flags) {
        Ok(prog) => {
            let compiled = Compiled { prog, pattern: pattern.into(), flags: flags_str.into(), global: flags.global, last_index: 0.0 };
            let handle = match FREE.with(|f| f.pop()) {
                Some(h) => {
                    TABLE.with(|t| t[(h as usize) - 1] = Some(compiled));
                    h as i32
                }
                None => TABLE.with(|t| {
                    t.push(Some(compiled));
                    t.len() as i32
                }),
            };
            box_handle(handle)
        }
        Err(e) => {
            LAST_ERROR.with(|s| {
                s.clear();
                s.push_str("invalid regular expression: ");
                s.push_str(error_message(e));
            });
            0
        }
    }
}

pub fn last_error() -> String {
    LAST_ERROR.read(|s| s.clone())
}

pub fn source(boxed: i32) -> String {
    with_compiled(unbox(boxed), |c| c.pattern.clone())
}

pub fn flags(boxed: i32) -> String {
    with_compiled(unbox(boxed), |c| c.flags.clone())
}

pub fn global(boxed: i32) -> bool {
    with_compiled(unbox(boxed), |c| c.global)
}

pub fn get_last_index(boxed: i32) -> f64 {
    with_compiled(unbox(boxed), |c| c.last_index)
}

pub fn set_last_index(boxed: i32, v: f64) {
    with_compiled_mut(unbox(boxed), |c| c.last_index = v);
}

/// Runs one search, applying/updating `lastIndex` exactly like JS does for
/// a `g` pattern (search starts there, and is reset to 0 on no match, or
/// advanced to the match's end on a match; a non-`g` pattern always
/// searches from the start and never touches `lastIndex`). Returns the
/// char-index captures plus the group count, or `None` for no match.
fn run(handle: i32, input: &str) -> Option<(crate::regex::Captures, u32)> {
    let (global, start16) = with_compiled(handle, |c| (c.global, c.last_index));
    let start_char = if global { utf16_to_char(input, start16.max(0.0) as usize) } else { 0 };
    if start_char > input.chars().count() {
        if global {
            with_compiled_mut(handle, |c| c.last_index = 0.0);
        }
        return None;
    }
    let n_groups = with_compiled(handle, |c| c.prog.group_count());
    let caps = with_compiled(handle, |c| crate::regex::exec(&c.prog, input, start_char));
    match caps {
        Some(caps) => {
            let (_, m_end) = caps.group(0).expect("group 0 always matches");
            if global {
                let new_last = char_to_utf16(input, m_end) as f64;
                with_compiled_mut(handle, |c| c.last_index = new_last);
            }
            Some((caps, n_groups))
        }
        None => {
            if global {
                with_compiled_mut(handle, |c| c.last_index = 0.0);
            }
            None
        }
    }
}

pub fn test(boxed: i32, input_ref: u32) -> bool {
    let input = strings::as_str(input_ref);
    run(unbox(boxed), input).is_some()
}

/// Returns a `T_ARR_REF` array of group strings (group 0 first), with a
/// null (`0`) entry for an unmatched group, or `0` for no match. Also
/// records the UTF-16 match-start index for `regex_last_match_index()`.
pub fn exec(boxed: i32, input_ref: u32) -> u32 {
    let input = strings::as_str(input_ref);
    match run(unbox(boxed), input) {
        Some((caps, n_groups)) => {
            let (m_start, _) = caps.group(0).expect("group 0 always matches");
            LAST_MATCH_INDEX.set(char_to_utf16(input, m_start) as f64);
            let arr = arrays::new(gc::T_ARR_REF, n_groups);
            for i in 0..n_groups {
                let s = match caps.group(i) {
                    Some((s, e)) => {
                        let byte_s = input.chars().take(s).map(char::len_utf8).sum::<usize>();
                        let byte_e = input.chars().take(e).map(char::len_utf8).sum::<usize>();
                        strings::from_str(&input[byte_s..byte_e]) as i32
                    }
                    None => 0,
                };
                arrays::push_i32(arr, s);
            }
            arr
        }
        None => 0,
    }
}

pub fn last_match_index() -> f64 {
    LAST_MATCH_INDEX.get()
}
