//! A JavaScript-flavored regular-expression matcher (step 1 of RegExp,
//! PRODUCTION.md L4).
//!
//! `parse` compiles a pattern into a [`Program`] (a Thompson NFA); `exec`
//! runs it with a Pike VM (a breadth-first simulation of all NFA threads at
//! once, each carrying its own capture slots). Because every reachable
//! instruction is visited at most once per input position, matching is
//! linear in `pattern.len() * input.len()`: no pattern can blow up
//! exponentially the way backtracking engines do (`(a+)+$` on a long run of
//! `a`s followed by a non-matching character finishes immediately; see the
//! tests below). The same "visit each pc once per step" rule is what stops
//! a zero-width loop body (`(a*)*`) from looping forever: once a `pc` is
//! marked visited at the current position, revisiting it during the same
//! `addthread` walk is a no-op.
//!
//! This module is pure logic: no Wasm exports, no compiler integration.
//! A later step wires it to `strings.rs` and the compiler.
//!
//! No `core::fmt`, no `format!`, no `HashMap`, no Unicode tables (HANDOFF.md
//! §6): character classes `\d \w \s` and case-insensitive matching are
//! ASCII-only, like a "non-unicode-mode" JS regex restricted to the ASCII
//! subset. `.` excludes only `\n` and `\r` (not the full JS line-terminator
//! set, which also includes U+2028/U+2029 — those would need Unicode
//! tables). Matching runs over `char`s (Unicode scalar values), not UTF-16
//! code units like `strings.rs`'s indices; a later step that wires this to
//! GC strings will need to translate between the two.
//!
//! Unsupported: backreferences (`\1`, `\2`, ...) and lookaround (`(?=`,
//! `(?!`, `(?<=`, `(?<!`) are rejected with [`ErrorKind::Unsupported`]
//! during parsing, so a later step can add them without changing this
//! module's public shape.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

/// Why `parse` rejected a pattern.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// A `(` with no matching `)`, or a dangling `(?`.
    UnterminatedGroup,
    /// A `[` with no matching `]`.
    UnterminatedClass,
    /// `{n,m}` with `n > m`, or a `{`/count that doesn't parse as digits.
    InvalidQuantifier,
    /// A quantifier (`* + ? {..}`) with nothing before it to repeat.
    QuantifierWithoutTarget,
    /// A trailing `\` with no character after it.
    TrailingEscape,
    /// A `)` with no matching `(`.
    UnmatchedCloseParen,
    /// Backreferences or lookaround: recognized syntax, not implemented yet.
    Unsupported,
}

/// Flags a caller passes to [`parse`]. `global` is not used by `exec`
/// (the caller drives repeated matches); it is carried here only so the
/// caller has one place to read all flags back from.
#[derive(Clone, Copy, Debug, Default)]
pub struct Flags {
    pub global: bool,
    /// ASCII case folding only (see module docs).
    pub ignore_case: bool,
    /// `^`/`$` also match right after/before a `\n`.
    pub multiline: bool,
    /// `.` also matches `\n` and `\r`.
    pub dot_all: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClassItem {
    Range(char, char),
    Digit,
    NotDigit,
    Word,
    NotWord,
    Space,
    NotSpace,
}

fn is_ascii_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn is_ascii_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c')
}

fn class_item_matches(item: &ClassItem, c: char, ignore_case: bool) -> bool {
    match *item {
        ClassItem::Range(lo, hi) => {
            if lo <= c && c <= hi {
                return true;
            }
            if ignore_case {
                let cl = ascii_fold(c);
                let (lo2, hi2) = (ascii_fold(lo), ascii_fold(hi));
                // Only ranges that are themselves plain ASCII letters fold
                // sensibly; approximate by folding the endpoints and the
                // candidate the same way.
                if lo2 <= cl && cl <= hi2 {
                    return true;
                }
            }
            false
        }
        ClassItem::Digit => c.is_ascii_digit(),
        ClassItem::NotDigit => !c.is_ascii_digit(),
        ClassItem::Word => is_ascii_word(c),
        ClassItem::NotWord => !is_ascii_word(c),
        ClassItem::Space => is_ascii_space(c),
        ClassItem::NotSpace => !is_ascii_space(c),
    }
}

fn ascii_fold(c: char) -> char {
    if c.is_ascii() { c.to_ascii_lowercase() } else { c }
}

#[derive(Clone, Debug)]
enum Node {
    Char(char),
    Any,
    Class(bool, Vec<ClassItem>),
    Start,
    End,
    WordBoundary(bool),
    Group(u32, Box<Node>),
    NonCapGroup(Box<Node>),
    Concat(Vec<Node>),
    Alt(Vec<Node>),
    Repeat(Box<Node>, u32, Option<u32>, bool),
    Empty,
}

struct Parser<'a> {
    chars: &'a [char],
    pos: usize,
    group_count: u32,
    names: Vec<(String, u32)>,
}

type PResult<T> = Result<T, ErrorKind>;

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, off: usize) -> Option<char> {
        self.chars.get(self.pos + off).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn parse_alt(&mut self) -> PResult<Node> {
        let mut branches = Vec::new();
        branches.push(self.parse_concat()?);
        while self.eat('|') {
            branches.push(self.parse_concat()?);
        }
        if branches.len() == 1 {
            Ok(branches.pop().unwrap())
        } else {
            Ok(Node::Alt(branches))
        }
    }

    fn parse_concat(&mut self) -> PResult<Node> {
        let mut parts = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            parts.push(self.parse_repeat()?);
        }
        if parts.is_empty() {
            Ok(Node::Empty)
        } else if parts.len() == 1 {
            Ok(parts.pop().unwrap())
        } else {
            Ok(Node::Concat(parts))
        }
    }

    fn parse_repeat(&mut self) -> PResult<Node> {
        let atom = self.parse_atom()?;
        self.parse_quantifier(atom)
    }

    fn parse_quantifier(&mut self, atom: Node) -> PResult<Node> {
        let (min, max): (u32, Option<u32>) = match self.peek() {
            Some('*') => {
                self.pos += 1;
                (0, None)
            }
            Some('+') => {
                self.pos += 1;
                (1, None)
            }
            Some('?') => {
                self.pos += 1;
                (0, Some(1))
            }
            Some('{') => {
                if let Some((min, max, consumed)) = self.try_parse_brace(self.pos) {
                    self.pos = consumed;
                    (min, max)
                } else {
                    return Ok(atom);
                }
            }
            _ => return Ok(atom),
        };
        if let (Some(mx), mn) = (max, min) {
            if mn > mx {
                return Err(ErrorKind::InvalidQuantifier);
            }
        }
        if matches!(atom, Node::Empty) {
            return Err(ErrorKind::QuantifierWithoutTarget);
        }
        let lazy = self.eat('?');
        Ok(Node::Repeat(Box::new(atom), min, max, lazy))
    }

    /// Tries to parse `{n}`, `{n,}` or `{n,m}` starting at `{` (index
    /// `start`, pointing at `{`). Returns `(min, max, pos_after)` on
    /// success; `None` if this isn't a valid brace quantifier (JS then
    /// treats `{` as a literal, which we don't special-case here since the
    /// caller only calls this when a quantifier is expected — a bare `{`
    /// with no valid count is simply left as a literal `Char('{')` already
    /// consumed by `parse_atom`, so we never get here with a stray `{`).
    fn try_parse_brace(&self, start: usize) -> Option<(u32, Option<u32>, usize)> {
        let mut i = start + 1;
        let n1_start = i;
        while self.chars.get(i).is_some_and(|c| c.is_ascii_digit()) {
            i += 1;
        }
        if i == n1_start {
            return None;
        }
        let min = digits_to_u32(&self.chars[n1_start..i]);
        if self.chars.get(i) == Some(&'}') {
            return Some((min, Some(min), i + 1));
        }
        if self.chars.get(i) != Some(&',') {
            return None;
        }
        i += 1;
        let n2_start = i;
        while self.chars.get(i).is_some_and(|c| c.is_ascii_digit()) {
            i += 1;
        }
        if self.chars.get(i) != Some(&'}') {
            return None;
        }
        let max = if i == n2_start { None } else { Some(digits_to_u32(&self.chars[n2_start..i])) };
        Some((min, max, i + 1))
    }

    fn parse_atom(&mut self) -> PResult<Node> {
        match self.peek() {
            None => Ok(Node::Empty),
            Some('(') => self.parse_group(),
            Some('[') => self.parse_class(),
            Some('.') => {
                self.pos += 1;
                Ok(Node::Any)
            }
            Some('^') => {
                self.pos += 1;
                Ok(Node::Start)
            }
            Some('$') => {
                self.pos += 1;
                Ok(Node::End)
            }
            Some('\\') => self.parse_escape(),
            Some('*') | Some('+') | Some('?') => Err(ErrorKind::QuantifierWithoutTarget),
            Some(c) => {
                self.pos += 1;
                Ok(Node::Char(c))
            }
        }
    }

    fn parse_group(&mut self) -> PResult<Node> {
        self.pos += 1; // '('
        if self.eat('?') {
            match self.peek() {
                Some(':') => {
                    self.pos += 1;
                    let inner = self.parse_alt()?;
                    if !self.eat(')') {
                        return Err(ErrorKind::UnterminatedGroup);
                    }
                    Ok(Node::NonCapGroup(Box::new(inner)))
                }
                Some('=') | Some('!') => Err(ErrorKind::Unsupported),
                Some('<') => {
                    // (?<name>...) or (?<=...) / (?<!...) lookbehind.
                    if matches!(self.peek_at(1), Some('=') | Some('!')) {
                        return Err(ErrorKind::Unsupported);
                    }
                    self.pos += 1; // '<'
                    let name_start = self.pos;
                    while matches!(self.peek(), Some(c) if c != '>') {
                        self.pos += 1;
                    }
                    if !self.eat('>') {
                        return Err(ErrorKind::UnterminatedGroup);
                    }
                    let name: String = self.chars[name_start..self.pos - 1].iter().collect();
                    self.group_count += 1;
                    let idx = self.group_count;
                    self.names.push((name, idx));
                    let inner = self.parse_alt()?;
                    if !self.eat(')') {
                        return Err(ErrorKind::UnterminatedGroup);
                    }
                    Ok(Node::Group(idx, Box::new(inner)))
                }
                _ => Err(ErrorKind::UnterminatedGroup),
            }
        } else {
            self.group_count += 1;
            let idx = self.group_count;
            let inner = self.parse_alt()?;
            if !self.eat(')') {
                return Err(ErrorKind::UnterminatedGroup);
            }
            Ok(Node::Group(idx, Box::new(inner)))
        }
    }

    fn parse_class(&mut self) -> PResult<Node> {
        self.pos += 1; // '['
        let negate = self.eat('^');
        let mut items = Vec::new();
        let mut first = true;
        loop {
            match self.peek() {
                None => return Err(ErrorKind::UnterminatedClass),
                Some(']') if !first => {
                    self.pos += 1;
                    break;
                }
                _ => {}
            }
            first = false;
            let lo = self.parse_class_atom()?;
            match lo {
                ClassAtom::Esc(item) => items.push(item),
                ClassAtom::Char(lo_c) => {
                    if self.peek() == Some('-') && self.peek_at(1) != Some(']') && self.peek_at(1).is_some() {
                        self.pos += 1; // '-'
                        match self.parse_class_atom()? {
                            ClassAtom::Char(hi_c) => {
                                if hi_c < lo_c {
                                    return Err(ErrorKind::InvalidQuantifier);
                                }
                                items.push(ClassItem::Range(lo_c, hi_c));
                            }
                            ClassAtom::Esc(_) => return Err(ErrorKind::InvalidQuantifier),
                        }
                    } else {
                        items.push(ClassItem::Range(lo_c, lo_c));
                    }
                }
            }
        }
        Ok(Node::Class(negate, items))
    }

    fn parse_class_atom(&mut self) -> PResult<ClassAtom> {
        if self.peek() == Some('\\') {
            self.pos += 1;
            let c = self.bump().ok_or(ErrorKind::TrailingEscape)?;
            Ok(match c {
                'd' => ClassAtom::Esc(ClassItem::Digit),
                'D' => ClassAtom::Esc(ClassItem::NotDigit),
                'w' => ClassAtom::Esc(ClassItem::Word),
                'W' => ClassAtom::Esc(ClassItem::NotWord),
                's' => ClassAtom::Esc(ClassItem::Space),
                'S' => ClassAtom::Esc(ClassItem::NotSpace),
                'n' => ClassAtom::Char('\n'),
                't' => ClassAtom::Char('\t'),
                'r' => ClassAtom::Char('\r'),
                'f' => ClassAtom::Char('\x0c'),
                'v' => ClassAtom::Char('\x0b'),
                'b' => ClassAtom::Char('\x08'),
                '0' => ClassAtom::Char('\0'),
                other if other.is_ascii_digit() => return Err(ErrorKind::Unsupported),
                other => ClassAtom::Char(other),
            })
        } else {
            let c = self.bump().ok_or(ErrorKind::UnterminatedClass)?;
            Ok(ClassAtom::Char(c))
        }
    }

    fn parse_escape(&mut self) -> PResult<Node> {
        self.pos += 1; // '\\'
        let c = self.bump().ok_or(ErrorKind::TrailingEscape)?;
        Ok(match c {
            'd' => Node::Class(false, [ClassItem::Digit].into()),
            'D' => Node::Class(false, [ClassItem::NotDigit].into()),
            'w' => Node::Class(false, [ClassItem::Word].into()),
            'W' => Node::Class(false, [ClassItem::NotWord].into()),
            's' => Node::Class(false, [ClassItem::Space].into()),
            'S' => Node::Class(false, [ClassItem::NotSpace].into()),
            'b' => Node::WordBoundary(false),
            'B' => Node::WordBoundary(true),
            'n' => Node::Char('\n'),
            't' => Node::Char('\t'),
            'r' => Node::Char('\r'),
            'f' => Node::Char('\x0c'),
            'v' => Node::Char('\x0b'),
            '0' => Node::Char('\0'),
            other if other.is_ascii_digit() => return Err(ErrorKind::Unsupported),
            other => Node::Char(other),
        })
    }
}

enum ClassAtom {
    Char(char),
    Esc(ClassItem),
}

fn digits_to_u32(chars: &[char]) -> u32 {
    let mut v: u32 = 0;
    for c in chars {
        v = v.saturating_mul(10).saturating_add((*c as u32) - ('0' as u32));
    }
    v
}

#[derive(Clone, Debug)]
enum Inst {
    Char(char),
    Any,
    Class(bool, Vec<ClassItem>),
    Save(usize),
    Jmp(usize),
    Split(usize, usize),
    Start,
    End,
    WordBoundary(bool),
    Match,
}

/// A compiled pattern, ready for repeated [`exec`] calls.
#[derive(Debug)]
pub struct Program {
    insts: Vec<Inst>,
    /// Number of capturing groups, including the implicit group 0 (the
    /// whole match).
    n_groups: u32,
    names: Vec<(String, u32)>,
    pub flags: Flags,
}

impl Program {
    /// The capture-group index for a named group (`(?<name>...)`), or
    /// `None` if the pattern has no group of that name.
    pub fn group_index(&self, name: &str) -> Option<u32> {
        self.names.iter().find(|(n, _)| n == name).map(|(_, i)| *i)
    }

    /// The number of capture groups, including the implicit group 0.
    pub fn group_count(&self) -> u32 {
        self.n_groups
    }
}

struct Compiler {
    insts: Vec<Inst>,
}

impl Compiler {
    fn emit(&mut self, node: &Node) {
        match node {
            Node::Empty => {}
            Node::Char(c) => self.insts.push(Inst::Char(*c)),
            Node::Any => self.insts.push(Inst::Any),
            Node::Class(neg, items) => self.insts.push(Inst::Class(*neg, items.clone())),
            Node::Start => self.insts.push(Inst::Start),
            Node::End => self.insts.push(Inst::End),
            Node::WordBoundary(b) => self.insts.push(Inst::WordBoundary(*b)),
            Node::NonCapGroup(inner) => self.emit(inner),
            Node::Group(idx, inner) => {
                self.insts.push(Inst::Save((*idx as usize) * 2));
                self.emit(inner);
                self.insts.push(Inst::Save((*idx as usize) * 2 + 1));
            }
            Node::Concat(parts) => {
                for p in parts {
                    self.emit(p);
                }
            }
            Node::Alt(branches) => self.emit_alt(branches),
            Node::Repeat(body, min, max, lazy) => self.emit_repeat(body, *min, *max, *lazy),
        }
    }

    fn emit_alt(&mut self, branches: &[Node]) {
        if branches.len() == 1 {
            self.emit(&branches[0]);
            return;
        }
        let split_pc = self.insts.len();
        self.insts.push(Inst::Split(0, 0)); // patched below
        self.emit(&branches[0]);
        let jmp_pc = self.insts.len();
        self.insts.push(Inst::Jmp(0)); // patched below
        let l2 = self.insts.len();
        self.insts[split_pc] = Inst::Split(split_pc + 1, l2);
        self.emit_alt(&branches[1..]);
        let end = self.insts.len();
        self.insts[jmp_pc] = Inst::Jmp(end);
    }

    fn emit_repeat(&mut self, body: &Node, min: u32, max: Option<u32>, lazy: bool) {
        match (min, max) {
            (0, None) => self.emit_star(body, lazy),
            (1, None) => self.emit_plus(body, lazy),
            (0, Some(1)) => self.emit_opt(body, lazy),
            (n, Some(m)) if n == m => {
                for _ in 0..n {
                    self.emit(body);
                }
            }
            (n, None) => {
                for _ in 0..n {
                    self.emit(body);
                }
                self.emit_star(body, lazy);
            }
            (n, Some(m)) => {
                for _ in 0..n {
                    self.emit(body);
                }
                self.emit_nested_opt(body, m - n, lazy);
            }
        }
    }

    fn emit_star(&mut self, body: &Node, lazy: bool) {
        let l1 = self.insts.len();
        let split_pc = l1;
        self.insts.push(Inst::Split(0, 0));
        let l2 = self.insts.len();
        self.emit(body);
        self.insts.push(Inst::Jmp(l1));
        let l3 = self.insts.len();
        self.insts[split_pc] = if lazy { Inst::Split(l3, l2) } else { Inst::Split(l2, l3) };
    }

    fn emit_plus(&mut self, body: &Node, lazy: bool) {
        let l1 = self.insts.len();
        self.emit(body);
        let split_pc = self.insts.len();
        self.insts.push(Inst::Split(0, 0));
        let l2 = self.insts.len();
        self.insts[split_pc] = if lazy { Inst::Split(l2, l1) } else { Inst::Split(l1, l2) };
    }

    fn emit_opt(&mut self, body: &Node, lazy: bool) {
        let split_pc = self.insts.len();
        self.insts.push(Inst::Split(0, 0));
        let l2 = self.insts.len();
        self.emit(body);
        let l3 = self.insts.len();
        self.insts[split_pc] = if lazy { Inst::Split(l3, l2) } else { Inst::Split(l2, l3) };
    }

    /// `body{n,n+extra}` after the mandatory `n` copies were already
    /// emitted: `extra` nested optionals, so `body?(body?(body?...))`,
    /// greedy-outer-first (or lazy-outer-first).
    fn emit_nested_opt(&mut self, body: &Node, extra: u32, lazy: bool) {
        if extra == 0 {
            return;
        }
        let split_pc = self.insts.len();
        self.insts.push(Inst::Split(0, 0));
        let l2 = self.insts.len();
        self.emit(body);
        self.emit_nested_opt(body, extra - 1, lazy);
        let l3 = self.insts.len();
        self.insts[split_pc] = if lazy { Inst::Split(l3, l2) } else { Inst::Split(l2, l3) };
    }
}

/// Parses a JS-flavored regex pattern (the part between the slashes, no
/// flag letters) into a [`Program`]. See the module docs for the supported
/// syntax and the ASCII-only limitations.
pub fn parse(pattern: &str, flags: Flags) -> Result<Program, ErrorKind> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut p = Parser { chars: &chars, pos: 0, group_count: 0, names: Vec::new() };
    let ast = p.parse_alt()?;
    if p.pos != p.chars.len() {
        // Only ')' can stop parse_alt early.
        return Err(ErrorKind::UnmatchedCloseParen);
    }
    let mut c = Compiler { insts: Vec::new() };
    c.insts.push(Inst::Save(0));
    c.emit(&ast);
    c.insts.push(Inst::Save(1));
    c.insts.push(Inst::Match);
    Ok(Program { insts: c.insts, n_groups: p.group_count + 1, names: p.names, flags })
}

/// Capture results of a successful [`exec`]. Slots hold `(start, end)` char
/// offsets into the input; group 0 is the whole match. An unmatched group
/// (never entered, e.g. the losing side of an alternation) is `None`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Captures {
    slots: Vec<Option<usize>>,
}

impl Captures {
    pub fn group(&self, i: u32) -> Option<(usize, usize)> {
        let s = self.slots.get((i as usize) * 2).copied().flatten();
        let e = self.slots.get((i as usize) * 2 + 1).copied().flatten();
        match (s, e) {
            (Some(s), Some(e)) => Some((s, e)),
            _ => None,
        }
    }
}

#[derive(Clone)]
struct Thread {
    pc: usize,
    caps: Vec<Option<usize>>,
}

struct ThreadList {
    threads: Vec<Thread>,
    visited_gen: Vec<u32>,
    generation: u32,
}

impl ThreadList {
    fn new(n: usize) -> Self {
        ThreadList { threads: Vec::new(), visited_gen: alloc::vec![0; n], generation: 0 }
    }

    fn start_step(&mut self) {
        self.generation += 1;
        self.threads.clear();
    }

    fn visit(&mut self, pc: usize) -> bool {
        if self.visited_gen[pc] == self.generation {
            false
        } else {
            self.visited_gen[pc] = self.generation;
            true
        }
    }
}

fn char_at(input: &[char], pos: usize) -> Option<char> {
    input.get(pos).copied()
}

fn at_word_boundary(input: &[char], pos: usize) -> bool {
    let before = pos.checked_sub(1).and_then(|i| char_at(input, i)).is_some_and(is_ascii_word);
    let after = char_at(input, pos).is_some_and(is_ascii_word);
    before != after
}

fn addthread(list: &mut ThreadList, insts: &[Inst], pc: usize, caps: Vec<Option<usize>>, input: &[char], pos: usize, flags: &Flags) {
    if !list.visit(pc) {
        return;
    }
    match &insts[pc] {
        Inst::Jmp(t) => addthread(list, insts, *t, caps, input, pos, flags),
        Inst::Split(a, b) => {
            addthread(list, insts, *a, caps.clone(), input, pos, flags);
            addthread(list, insts, *b, caps, input, pos, flags);
        }
        Inst::Save(slot) => {
            let mut caps = caps;
            if *slot >= caps.len() {
                caps.resize(slot + 1, None);
            }
            caps[*slot] = Some(pos);
            addthread(list, insts, pc + 1, caps, input, pos, flags);
        }
        Inst::Start => {
            let ok = pos == 0 || (flags.multiline && char_at(input, pos - 1) == Some('\n'));
            if ok {
                addthread(list, insts, pc + 1, caps, input, pos, flags);
            }
        }
        Inst::End => {
            let ok = pos == input.len() || (flags.multiline && char_at(input, pos) == Some('\n'));
            if ok {
                addthread(list, insts, pc + 1, caps, input, pos, flags);
            }
        }
        Inst::WordBoundary(negate) => {
            if at_word_boundary(input, pos) != *negate {
                addthread(list, insts, pc + 1, caps, input, pos, flags);
            }
        }
        _ => list.threads.push(Thread { pc, caps }),
    }
}

fn char_matches(c: char, want: char, ignore_case: bool) -> bool {
    c == want || (ignore_case && ascii_fold(c) == ascii_fold(want))
}

/// Finds the leftmost match of `prog` in `input`, starting the search no
/// earlier than char offset `start` (the caller advances `start` between
/// calls for a global search; `exec` itself always looks for the leftmost
/// match at or after `start`, matching JS's `lastIndex`-driven search).
pub fn exec(prog: &Program, input: &str, start: usize) -> Option<Captures> {
    let chars: Vec<char> = input.chars().collect();
    exec_chars(prog, &chars, start)
}

fn exec_chars(prog: &Program, input: &[char], start: usize) -> Option<Captures> {
    let insts = &prog.insts;
    let n = insts.len();
    let mut clist = ThreadList::new(n);
    let mut nlist = ThreadList::new(n);
    let mut matched: Option<Vec<Option<usize>>> = None;

    let empty_caps = alloc::vec![None; (prog.n_groups as usize) * 2];
    if start > input.len() {
        return None;
    }

    clist.start_step();

    let mut pos = start;
    loop {
        // Add a fresh start thread at this position, at the lowest
        // priority (appended after whatever is already running from an
        // earlier, more-leftmost start). Once something has matched, no
        // later start can be more leftmost, so we stop seeding new ones.
        if matched.is_none() {
            addthread(&mut clist, insts, 0, empty_caps.clone(), input, pos, &prog.flags);
        }

        if clist.threads.is_empty() {
            // Nothing alive at this position. If we've already matched,
            // there is nothing left that could improve on it. Otherwise,
            // reset for the next position's seed and keep looking.
            if matched.is_some() || pos >= input.len() {
                break;
            }
            clist.start_step();
            pos += 1;
            continue;
        }

        nlist.start_step();
        let c = char_at(input, pos);
        for t in clist.threads.clone() {
            match &insts[t.pc] {
                Inst::Char(want) => {
                    if let Some(c) = c {
                        if char_matches(c, *want, prog.flags.ignore_case) {
                            addthread(&mut nlist, insts, t.pc + 1, t.caps, input, pos + 1, &prog.flags);
                        }
                    }
                }
                Inst::Any => {
                    if let Some(c) = c {
                        let excluded = !prog.flags.dot_all && (c == '\n' || c == '\r');
                        if !excluded {
                            addthread(&mut nlist, insts, t.pc + 1, t.caps, input, pos + 1, &prog.flags);
                        }
                    }
                }
                Inst::Class(negate, items) => {
                    if let Some(c) = c {
                        let m = items.iter().any(|it| class_item_matches(it, c, prog.flags.ignore_case));
                        if m != *negate {
                            addthread(&mut nlist, insts, t.pc + 1, t.caps, input, pos + 1, &prog.flags);
                        }
                    }
                }
                Inst::Match => {
                    matched = Some(t.caps);
                    break; // lower-priority threads this step lose.
                }
                _ => unreachable!("zero-width instructions are resolved in addthread"),
            }
        }

        if pos >= input.len() {
            break;
        }
        pos += 1;
        core::mem::swap(&mut clist, &mut nlist);
    }

    matched.map(|slots| Captures { slots })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prog(pattern: &str) -> Program {
        parse(pattern, Flags::default()).unwrap()
    }

    fn prog_flags(pattern: &str, flags: Flags) -> Program {
        parse(pattern, flags).unwrap()
    }

    fn m(pattern: &str, input: &str) -> Option<(usize, usize)> {
        exec(&prog(pattern), input, 0).and_then(|c| c.group(0))
    }

    #[test]
    fn literal_chars() {
        assert_eq!(m("abc", "xxabcxx"), Some((2, 5)));
        assert_eq!(m("abc", "xxx"), None);
    }

    #[test]
    fn dot() {
        assert_eq!(m("a.c", "abc"), Some((0, 3)));
        assert_eq!(m("a.c", "a\nc"), None);
        let p = prog_flags("a.c", Flags { dot_all: true, ..Default::default() });
        assert_eq!(exec(&p, "a\nc", 0).and_then(|c| c.group(0)), Some((0, 3)));
    }

    #[test]
    fn classes_and_ranges() {
        assert_eq!(m("[abc]+", "xxabccbaxx"), Some((2, 8)));
        assert_eq!(m("[a-z]+", "ABCdefGHI"), Some((3, 6)));
        assert_eq!(m("[^a-z]+", "abcDEFghi"), Some((3, 6)));
    }

    #[test]
    fn escapes_classes() {
        assert_eq!(m(r"\d+", "ab123cd"), Some((2, 5)));
        assert_eq!(m(r"\D+", "123abc456"), Some((3, 6)));
        assert_eq!(m(r"\w+", "  hi_there  "), Some((2, 10)));
        assert_eq!(m(r"\W+", "ab   cd"), Some((2, 5)));
        assert_eq!(m(r"\s+", "ab  cd"), Some((2, 4)));
        assert_eq!(m(r"\S+", "  ab  "), Some((2, 4)));
        assert_eq!(m(r"a\.b", "a.b"), Some((0, 3)));
        assert_eq!(m(r"a\/b", "a/b"), Some((0, 3)));
        assert_eq!(m(r"a\\b", r"a\b"), Some((0, 3)));
        assert_eq!(m(r"\n\t\r", "\n\t\r"), Some((0, 3)));
    }

    #[test]
    fn anchors() {
        assert_eq!(m("^abc", "abcxx"), Some((0, 3)));
        assert_eq!(m("^abc", "xabcxx"), None);
        assert_eq!(m("abc$", "xxabc"), Some((2, 5)));
        assert_eq!(m("abc$", "xxabcx"), None);
    }

    #[test]
    fn multiline_flag() {
        let p = prog("^b");
        assert_eq!(exec(&p, "a\nb", 0).and_then(|c| c.group(0)), None);
        let pm = prog_flags("^b", Flags { multiline: true, ..Default::default() });
        assert_eq!(exec(&pm, "a\nb", 0).and_then(|c| c.group(0)), Some((2, 3)));
        let pm2 = prog_flags("a$", Flags { multiline: true, ..Default::default() });
        assert_eq!(exec(&pm2, "a\nb", 0).and_then(|c| c.group(0)), Some((0, 1)));
    }

    #[test]
    fn ignore_case_flag() {
        let p = prog_flags("abc", Flags { ignore_case: true, ..Default::default() });
        assert_eq!(exec(&p, "XXABCXX", 0).and_then(|c| c.group(0)), Some((2, 5)));
        let pc = prog_flags("[a-z]+", Flags { ignore_case: true, ..Default::default() });
        assert_eq!(exec(&pc, "ABCdef", 0).and_then(|c| c.group(0)), Some((0, 6)));
    }

    #[test]
    fn word_boundary() {
        assert_eq!(m(r"\bcat\b", "a cat sat"), Some((2, 5)));
        assert_eq!(m(r"\bcat\b", "concatenate"), None);
        assert_eq!(m(r"\Bcat\B", "concatenate"), Some((3, 6)));
    }

    #[test]
    fn groups_and_captures() {
        let p = prog("(a)(b)(c)");
        let caps = exec(&p, "abc", 0).unwrap();
        assert_eq!(caps.group(0), Some((0, 3)));
        assert_eq!(caps.group(1), Some((0, 1)));
        assert_eq!(caps.group(2), Some((1, 2)));
        assert_eq!(caps.group(3), Some((2, 3)));
    }

    #[test]
    fn non_capturing_group() {
        let p = prog("(?:ab)+(c)");
        let caps = exec(&p, "ababc", 0).unwrap();
        assert_eq!(caps.group(0), Some((0, 5)));
        assert_eq!(caps.group(1), Some((4, 5)));
    }

    #[test]
    fn named_group() {
        let p = prog("(?<year>[0-9]+)-(?<month>[0-9]+)");
        assert_eq!(p.group_index("year"), Some(1));
        assert_eq!(p.group_index("month"), Some(2));
        assert_eq!(p.group_index("nope"), None);
        let caps = exec(&p, "2024-10", 0).unwrap();
        assert_eq!(caps.group(1), Some((0, 4)));
        assert_eq!(caps.group(2), Some((5, 7)));
    }

    #[test]
    fn unmatched_group_is_none() {
        let p = prog("(a)|(b)");
        let caps = exec(&p, "b", 0).unwrap();
        assert_eq!(caps.group(1), None);
        assert_eq!(caps.group(2), Some((0, 1)));
    }

    #[test]
    fn last_iteration_wins_for_repeated_group() {
        // JS: /(a)+/.exec("aaa")[1] === "a" (the last iteration's capture).
        let p = prog("(a)+");
        let caps = exec(&p, "aaa", 0).unwrap();
        assert_eq!(caps.group(0), Some((0, 3)));
        assert_eq!(caps.group(1), Some((2, 3)));
    }

    #[test]
    fn quantifiers() {
        assert_eq!(m("a*", "aaa"), Some((0, 3)));
        assert_eq!(m("a*", "bbb"), Some((0, 0)));
        assert_eq!(m("a+", "bbb"), None);
        assert_eq!(m("a+", "aaab"), Some((0, 3)));
        assert_eq!(m("colou?r", "color"), Some((0, 5)));
        assert_eq!(m("colou?r", "colour"), Some((0, 6)));
        assert_eq!(m("a{3}", "aaaa"), Some((0, 3)));
        assert_eq!(m("a{2,}", "aaaa"), Some((0, 4)));
        assert_eq!(m("a{2,3}", "aaaa"), Some((0, 3)));
        assert_eq!(m("a{0,2}b", "aaab"), Some((1, 4)));
        assert_eq!(m("a{0,2}b", "aab"), Some((0, 3)));
    }

    #[test]
    fn lazy_vs_greedy() {
        assert_eq!(m("a.*b", "axbxb"), Some((0, 5)));
        assert_eq!(m("a.*?b", "axbxb"), Some((0, 3)));
        assert_eq!(m("a+", "aaa"), Some((0, 3)));
        assert_eq!(m("a+?", "aaa"), Some((0, 1)));
        assert_eq!(m("a{1,3}?", "aaa"), Some((0, 1)));
        assert_eq!(m("a??", "aaa"), Some((0, 0)));
    }

    #[test]
    fn alternation_priority() {
        // JS: /a|ab/.exec("ab")[0] === "a" (leftmost ALTERNATIVE wins, not
        // leftmost-longest).
        assert_eq!(m("a|ab", "ab"), Some((0, 1)));
        assert_eq!(m("ab|a", "ab"), Some((0, 2)));
    }

    #[test]
    fn empty_pattern() {
        assert_eq!(m("", "abc"), Some((0, 0)));
    }

    #[test]
    fn pathological_pattern_is_fast() {
        // A backtracking engine takes exponential time on this; the Pike
        // VM here is linear, so this must return essentially instantly.
        let mut input = String::new();
        for _ in 0..30 {
            input.push('a');
        }
        input.push('b');
        assert_eq!(m("(a+)+$", &input), None);

        let mut input2 = String::new();
        for _ in 0..30 {
            input2.push('a');
        }
        assert_eq!(m("(a+)+$", &input2), Some((0, 30)));
    }

    #[test]
    fn empty_loop_terminates() {
        // (a*)* must not loop forever on an empty-body iteration.
        assert_eq!(m("(a*)*b", "aaab"), Some((0, 4)));
        assert_eq!(m("(a*)*b", "b"), Some((0, 1)));
        assert_eq!(m("(a*)*", ""), Some((0, 0)));
    }

    #[test]
    fn parse_errors() {
        assert_eq!(parse("(", Flags::default()).unwrap_err(), ErrorKind::UnterminatedGroup);
        assert_eq!(parse("[", Flags::default()).unwrap_err(), ErrorKind::UnterminatedClass);
        assert_eq!(parse("a{2,1}", Flags::default()).unwrap_err(), ErrorKind::InvalidQuantifier);
        assert_eq!(parse("*a", Flags::default()).unwrap_err(), ErrorKind::QuantifierWithoutTarget);
        assert_eq!(parse(")", Flags::default()).unwrap_err(), ErrorKind::UnmatchedCloseParen);
    }

    #[test]
    fn unsupported_syntax_rejected() {
        assert_eq!(parse("(?=abc)", Flags::default()).unwrap_err(), ErrorKind::Unsupported);
        assert_eq!(parse("(?!abc)", Flags::default()).unwrap_err(), ErrorKind::Unsupported);
        assert_eq!(parse("(?<=abc)", Flags::default()).unwrap_err(), ErrorKind::Unsupported);
        assert_eq!(parse("(?<!abc)", Flags::default()).unwrap_err(), ErrorKind::Unsupported);
        assert_eq!(parse(r"(a)\1", Flags::default()).unwrap_err(), ErrorKind::Unsupported);
    }
}
