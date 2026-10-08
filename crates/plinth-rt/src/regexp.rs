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

// -- String methods with a RegExp (core 1.12, PRODUCTION.md L4 step 3) -----
//
// `s.match`, `matchAll`, `replace`, `replaceAll`, `split` and `search` with
// a `RegExp` argument, with the JavaScript semantics. Each function splits
// the input into chars one time (`Text`) and runs the matcher on that.

/// An input string split into chars, with the byte and UTF-16 offset of
/// each char (and of the end).
struct Text<'a> {
    s: &'a str,
    chars: Vec<char>,
    byte_at: Vec<usize>,
    u16_at: Vec<usize>,
}

impl<'a> Text<'a> {
    fn new(s: &'a str) -> Self {
        let mut chars = Vec::new();
        let mut byte_at = Vec::new();
        let mut u16_at = Vec::new();
        let mut u = 0;
        for (b, c) in s.char_indices() {
            chars.push(c);
            byte_at.push(b);
            u16_at.push(u);
            u += c.len_utf16();
        }
        byte_at.push(s.len());
        u16_at.push(u);
        Text { s, chars, byte_at, u16_at }
    }

    fn len(&self) -> usize {
        self.chars.len()
    }

    /// The text between two char indices.
    fn sub(&self, a: usize, b: usize) -> &'a str {
        &self.s[self.byte_at[a]..self.byte_at[b]]
    }

    fn u16(&self, c: usize) -> usize {
        self.u16_at[c]
    }

    /// The char index of a UTF-16 offset (rounded up inside a surrogate
    /// pair, clamped to the end).
    fn char_of_u16(&self, u: usize) -> usize {
        match self.u16_at.binary_search(&u) {
            Ok(i) => i,
            Err(i) => i.min(self.len()),
        }
    }

    fn group(&self, caps: &crate::regex::Captures, i: u32) -> Option<&'a str> {
        caps.group(i).map(|(a, b)| self.sub(a, b))
    }
}

/// All matches from char `start` (for a `g` pattern), or the first match
/// from 0 (for another pattern). After an empty match the search moves on
/// by one char, as JavaScript's `AdvanceStringIndex` does, so it ends.
fn find_all(prog: &crate::regex::Program, text: &Text, start: usize, all: bool) -> Vec<crate::regex::Captures> {
    let mut out = Vec::new();
    if !all {
        if let Some(c) = crate::regex::exec_chars(prog, &text.chars, 0) {
            out.push(c);
        }
        return out;
    }
    let mut pos = start;
    while pos <= text.len() {
        let Some(c) = crate::regex::exec_chars(prog, &text.chars, pos) else { break };
        let (s, e) = c.group(0).expect("group 0 always matches");
        pos = if e == s { e + 1 } else { e };
        out.push(c);
    }
    out
}

fn set_error(msg: &str) {
    LAST_ERROR.with(|e| {
        e.clear();
        e.push_str(msg);
    });
}

/// The `(string | null)[]` of one match: the whole match, then each group.
fn groups_array(text: &Text, caps: &crate::regex::Captures, n_groups: u32) -> u32 {
    let arr = arrays::new(gc::T_ARR_REF, n_groups);
    for i in 0..n_groups {
        let s = text.group(caps, i).map_or(0, |g| strings::from_str(g) as i32);
        arrays::push_i32(arr, s);
    }
    arr
}

/// `s.search(re)`: the UTF-16 index of the first match, or -1. It does not
/// read or change `lastIndex`.
pub fn search(boxed: i32, input_ref: u32) -> f64 {
    let text = Text::new(strings::as_str(input_ref));
    let caps = with_compiled(unbox(boxed), |c| crate::regex::exec_chars(&c.prog, &text.chars, 0));
    match caps {
        Some(c) => text.u16(c.group(0).expect("group 0 always matches").0) as f64,
        None => -1.0,
    }
}

/// `s.match(re)` for a `g` pattern: a `string[]` of all whole matches, or
/// `0` (null) when there is none. `lastIndex` is 0 after it, as in JS.
pub fn match_global(boxed: i32, input_ref: u32) -> u32 {
    let text = Text::new(strings::as_str(input_ref));
    let handle = unbox(boxed);
    let found = with_compiled(handle, |c| find_all(&c.prog, &text, 0, true));
    with_compiled_mut(handle, |c| c.last_index = 0.0);
    if found.is_empty() {
        return 0;
    }
    let arr = arrays::new(gc::T_ARR_REF, found.len() as u32);
    for caps in &found {
        let m = text.group(caps, 0).expect("group 0 always matches");
        arrays::push_i32(arr, strings::from_str(m) as i32);
    }
    arr
}

static SCAN_INDICES: Global<Vec<f64>> = Global::new(Vec::new());

/// The matches for `matchAll` and for `replace` with a function, as an
/// array of `(string | null)[]` group arrays (like `exec`'s `groups`). The
/// UTF-16 start index of each match is kept for `scan_indices`. `mode`:
/// 0 = `replace` (all matches with `g`, then `lastIndex` is 0; else the
/// first match), 1 = `matchAll` (needs `g`; starts at `lastIndex` and does
/// not change it), 2 = `replaceAll` (needs `g`, else as 0). When the
/// pattern needs `g` and has not got it, returns `0` with the message in
/// `last_error()`.
pub fn scan(boxed: i32, input_ref: u32, mode: i32) -> u32 {
    let text = Text::new(strings::as_str(input_ref));
    let handle = unbox(boxed);
    let (global, last, n_groups) = with_compiled(handle, |c| (c.global, c.last_index, c.prog.group_count()));
    if mode != 0 && !global {
        set_error(if mode == 1 { "matchAll needs a RegExp with the g flag" } else { "replaceAll needs a RegExp with the g flag" });
        return 0;
    }
    let start = if mode == 1 { text.char_of_u16(last.max(0.0) as usize) } else { 0 };
    let found = with_compiled(handle, |c| find_all(&c.prog, &text, start, global));
    if global && mode != 1 {
        with_compiled_mut(handle, |c| c.last_index = 0.0);
    }
    SCAN_INDICES.with(|v| {
        v.clear();
        v.extend(found.iter().map(|caps| text.u16(caps.group(0).expect("group 0 always matches").0) as f64));
    });
    let arr = arrays::new(gc::T_ARR_REF, found.len() as u32);
    for caps in &found {
        arrays::push_i32(arr, groups_array(&text, caps, n_groups) as i32);
    }
    arr
}

/// The UTF-16 start indices of the matches of the last `scan`, as a
/// `number[]`. Generated code reads it at once after `scan`.
pub fn scan_indices() -> u32 {
    let arr = arrays::new(gc::T_ARR_F64, SCAN_INDICES.read(|v| v.len()) as u32);
    SCAN_INDICES.read(|v| {
        for x in v {
            arrays::push_f64(arr, *x);
        }
    });
    arr
}

/// `replace` with a function: puts `reps[i]` in place of match `i` of a
/// `scan` (`groups[i][0]` at UTF-16 index `indices[i]`).
pub fn splice(input_ref: u32, groups: u32, indices: u32, reps: u32) -> u32 {
    let text = Text::new(strings::as_str(input_ref));
    let mut out = String::new();
    let mut next = 0;
    for i in 0..arrays::len(groups) as i32 {
        let whole = strings::as_str(arrays::get_i32(arrays::get_i32(groups, i) as u32, 0) as u32);
        let at = text.char_of_u16(arrays::get_f64(indices, i) as usize);
        if at < next {
            continue;
        }
        out.push_str(text.sub(next, at));
        out.push_str(strings::as_str(arrays::get_i32(reps, i) as u32));
        next = (at + whole.chars().count()).min(text.len());
    }
    out.push_str(text.sub(next, text.len()));
    strings::from_str(&out)
}

/// Appends the replacement text `repl` for one match, with the patterns
/// `$$`, `$&`, `` $` ``, `$'`, `$1`..`$99` and `$<name>` (JavaScript's
/// `GetSubstitution`). An unmatched group gives "".
fn substitute(out: &mut String, repl: &str, text: &Text, caps: &crate::regex::Captures, prog: &crate::regex::Program) {
    let (ms, me) = caps.group(0).expect("group 0 always matches");
    let m = prog.group_count() - 1;
    let b = repl.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'$' || i + 1 >= b.len() {
            let end = repl[i + 1..].find('$').map_or(b.len(), |k| i + 1 + k);
            out.push_str(&repl[i..end]);
            i = end;
            continue;
        }
        match b[i + 1] {
            b'$' => {
                out.push('$');
                i += 2;
            }
            b'&' => {
                out.push_str(text.sub(ms, me));
                i += 2;
            }
            b'`' => {
                out.push_str(text.sub(0, ms));
                i += 2;
            }
            b'\'' => {
                out.push_str(text.sub(me, text.len()));
                i += 2;
            }
            d @ b'0'..=b'9' => {
                let d1 = (d - b'0') as u32;
                let two = b.get(i + 2).filter(|c| c.is_ascii_digit()).map(|c| d1 * 10 + (c - b'0') as u32);
                match two {
                    Some(n) if n >= 1 && n <= m => {
                        out.push_str(text.group(caps, n).unwrap_or(""));
                        i += 3;
                    }
                    _ if d1 >= 1 && d1 <= m => {
                        out.push_str(text.group(caps, d1).unwrap_or(""));
                        i += 2;
                    }
                    _ => {
                        out.push('$');
                        i += 1;
                    }
                }
            }
            b'<' if prog.has_names() => match repl[i + 2..].find('>') {
                Some(k) => {
                    let name = &repl[i + 2..i + 2 + k];
                    if let Some(g) = prog.group_index(name) {
                        out.push_str(text.group(caps, g).unwrap_or(""));
                    }
                    i += 2 + k + 1;
                }
                None => {
                    out.push_str("$<");
                    i += 2;
                }
            },
            _ => {
                out.push('$');
                i += 1;
            }
        }
    }
}

/// `s.replace(re, replacement)` and `s.replaceAll(re, replacement)` with a
/// replacement string. With `g`, every match is replaced and `lastIndex`
/// is 0 after; without it, only the first match is. `require_g` (for
/// `replaceAll`) makes a pattern without `g` an error: returns `0` with the
/// message in `last_error()`.
pub fn replace(boxed: i32, input_ref: u32, repl_ref: u32, require_g: bool) -> u32 {
    let repl = strings::as_str(repl_ref);
    let text = Text::new(strings::as_str(input_ref));
    let handle = unbox(boxed);
    let global = with_compiled(handle, |c| c.global);
    if require_g && !global {
        set_error("replaceAll needs a RegExp with the g flag");
        return 0;
    }
    let out = with_compiled(handle, |c| replace_text(&c.prog, &text, repl, global));
    if global {
        with_compiled_mut(handle, |c| c.last_index = 0.0);
    }
    strings::from_str(&out)
}

/// The result of `replace` with a replacement string.
fn replace_text(prog: &crate::regex::Program, text: &Text, repl: &str, global: bool) -> String {
    let mut out = String::new();
    let mut next = 0;
    for caps in &find_all(prog, text, 0, global) {
        let (s, e) = caps.group(0).expect("group 0 always matches");
        out.push_str(text.sub(next, s));
        substitute(&mut out, repl, text, caps, prog);
        next = e;
    }
    out.push_str(text.sub(next, text.len()));
    out
}

/// JavaScript's `ToUint32`, for the `limit` of `split`.
fn to_uint32(x: f64) -> u32 {
    if !x.is_finite() {
        return 0;
    }
    (x as i64) as u32
}

/// The parts of `split` (`None` for an unmatched group).
fn split_text<'a>(prog: &crate::regex::Program, text: &Text<'a>, lim: usize) -> Vec<Option<&'a str>> {
    let mut parts = Vec::new();
    if lim == 0 {
        return parts;
    }
    let size = text.len();
    if size == 0 {
        if crate::regex::exec_chars(prog, &text.chars, 0).is_none() {
            parts.push(Some(text.s));
        }
        return parts;
    }
    let n_groups = prog.group_count();
    let mut p = 0;
    let mut q = 0;
    while q < size {
        let Some(caps) = crate::regex::exec_chars(prog, &text.chars, q) else { break };
        let (ms, e) = caps.group(0).expect("group 0 always matches");
        if ms >= size {
            break;
        }
        if e == p {
            q = ms + 1;
            continue;
        }
        parts.push(Some(text.sub(p, ms)));
        if parts.len() == lim {
            return parts;
        }
        p = e;
        for i in 1..n_groups {
            parts.push(text.group(&caps, i));
            if parts.len() == lim {
                return parts;
            }
        }
        q = p;
    }
    parts.push(Some(text.sub(p, size)));
    parts
}

/// `s.split(re, limit)` (JavaScript's `@@split`): the parts between the
/// matches, with the groups of each match put in between (an unmatched
/// group gives "", where JavaScript gives `undefined`). An empty match
/// where the last part starts does not split. `g` and `lastIndex` have no
/// effect, and `lastIndex` does not change.
pub fn split(boxed: i32, input_ref: u32, limit: f64) -> u32 {
    let text = Text::new(strings::as_str(input_ref));
    let lim = to_uint32(limit) as usize;
    let parts = with_compiled(unbox(boxed), |c| split_text(&c.prog, &text, lim));
    let arr = arrays::new(gc::T_ARR_REF, parts.len() as u32);
    for part in parts {
        arrays::push_i32(arr, strings::from_str(part.unwrap_or("")) as i32);
    }
    arr
}


#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    fn prog(pattern: &str, flags: &str) -> crate::regex::Program {
        crate::regex::parse(pattern, flags_from_str(flags).unwrap()).unwrap()
    }

    fn rep(pattern: &str, flags: &str, input: &str, repl: &str) -> String {
        replace_text(&prog(pattern, flags), &Text::new(input), repl, flags.contains('g'))
    }

    fn spl(pattern: &str, input: &str, limit: f64) -> Vec<String> {
        let text = Text::new(input);
        split_text(&prog(pattern, ""), &text, to_uint32(limit) as usize).into_iter().map(|p| p.unwrap_or("").to_string()).collect()
    }

    #[test]
    fn replace_patterns() {
        assert_eq!(rep("x*", "g", "abc", "-"), "-a-b-c-");
        assert_eq!(rep("b", "", "abcb", "[$&]"), "a[b]cb");
        assert_eq!(rep("b", "g", "abcb", "[$`|$']"), "a[a|cb]c[abc|]");
        assert_eq!(rep("(\\w)(\\d)", "g", "a1 b2", "$2$1$$"), "1a$ 2b$");
        assert_eq!(rep("(a)", "", "a", "$01$10$0$3$"), "aa0$0$3$");
        assert_eq!(rep("(?<y>\\d+)", "", "x 42", "<$<y>>"), "x <42>");
        assert_eq!(rep("(?<y>\\d+)", "", "x 42", "$<z>|$<y"), "x |$<y");
        assert_eq!(rep("(\\d+)", "", "x 42", "$<y>"), "x $<y>");
        assert_eq!(rep("(a)|(b)", "g", "ab", "[$2]"), "[][b]");
        assert_eq!(rep("é", "g", "héhé", "e"), "hehe");
        assert_eq!(rep("", "g", "😀x", "."), ".😀.x.");
    }

    #[test]
    fn split_like_javascript() {
        let none = 4294967295.0;
        assert_eq!(spl(",", "a,b,,c", none), ["a", "b", "", "c"]);
        assert_eq!(spl("", "abc", none), ["a", "b", "c"]);
        assert_eq!(spl("x*", "abc", none), ["a", "b", "c"]);
        assert_eq!(spl("(-)", "a-b", none), ["a", "-", "b"]);
        assert_eq!(spl(",", "a,b,c", 2.0), ["a", "b"]);
        assert_eq!(spl(",", "a,b,c", 0.0), Vec::<String>::new());
        assert_eq!(spl(",", "a,b,c", -1.0), ["a", "b", "c"]);
        assert_eq!(spl(",", "", none), [""]);
        assert!(spl("", "", none).is_empty());
        assert_eq!(spl("(x)?-", "a-b", none), ["a", "", "b"]);
        assert_eq!(spl("\\s*,\\s*", "a , b,c", none), ["a", "b", "c"]);
        assert_eq!(spl("b*", "abbc", none), ["a", "c"]);
    }

    #[test]
    fn text_offsets() {
        let t = Text::new("a😀b");
        assert_eq!(t.u16(2), 3);
        assert_eq!(t.char_of_u16(3), 2);
        assert_eq!(t.char_of_u16(2), 2);
        assert_eq!(t.char_of_u16(99), 3);
        assert_eq!(t.sub(1, 2), "😀");
    }
}
