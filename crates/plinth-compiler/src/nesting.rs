//! A guard that runs before the parser (docs/GAPS.md R3).
//!
//! oxc parses nested code with recursion, so code that nests very deeply
//! (100,000 parentheses) overflows the stack before the converter in
//! `parse.rs` can report `parse::MAX_DEPTH`. This scan estimates the
//! nesting from the tokens, with a loop and no recursion, and stops code
//! that nests far too deep (`SCAN_LIMIT`) before oxc sees it.
//!
//! The estimate is high, not low: each open bracket, JSX element and
//! template counts one level, and in each bracket the tokens that can
//! start a nested part (a prefix operator, `?`, `=`, `=>`, `**`, `<`,
//! `if`, `else`, ...) count one level each until the part surely ends
//! (`;`, `,`, the closing bracket). A left-deep chain (`a + b + c`,
//! `x.f().g()`) counts no levels, because oxc parses it with a loop.
//! Real code stays far below the limit; code that is near the limit is
//! rejected later by `parse::MAX_DEPTH` in any case.

/// The estimate at which the scan stops the code. It is far above
/// `parse::MAX_DEPTH`, so the scan does not stop code that the parser
/// accepts, and far below the nesting that overflows the stack of a
/// compile (`crate::STACK_SIZE`): a debug build of oxc parses 50,000
/// levels of each kind of nesting on that stack.
pub const SCAN_LIMIT: u32 = 20 * crate::parse::MAX_DEPTH;

/// A part of the source that the scan is in.
enum Mode {
    /// Code, up to the byte `close` (none at the top level).
    Code(Frame),
    /// A template literal, after a backtick or the `}` of a `${`.
    Template,
    /// A JSX tag, after `<` and before its `>` or `/>`.
    JsxTag,
    /// The children of a JSX element, before its closing tag.
    JsxChildren,
}

/// The levels that one bracket of code adds to the estimate.
struct Frame {
    close: u8,
    /// Prefix operators, `?`, `=`, `=>`, `**` and `...` since the last
    /// `;` or `,`.
    run: u32,
    /// `if`, `for`, `while`, `with` and `do` since the last `;` or block.
    stmts: u32,
    /// `<` (a type argument, a comparison) since the last `;`, less the
    /// `>` that close one.
    angles: u32,
    /// `else` in the current `if` chain.
    elses: u32,
    /// A statement ended (`;` or a block): an `else` may continue it.
    after_stmt: bool,
}

impl Frame {
    fn new(close: u8) -> Self {
        Frame { close, run: 0, stmts: 0, angles: 0, elses: 0, after_stmt: false }
    }

    fn levels(&self) -> u32 {
        1 + self.run + self.stmts + self.angles + self.elses
    }
}

struct Scan {
    modes: Vec<Mode>,
    /// The estimate: the sum of the levels of all modes.
    total: u32,
    /// The last token ended an operand (a name, a literal, a closing
    /// bracket), so `-`, `/` and `<` after it are binary operators.
    operand: bool,
}

impl Scan {
    fn push(&mut self, m: Mode) {
        self.total += match &m {
            Mode::Code(f) => f.levels(),
            _ => 1,
        };
        self.modes.push(m);
    }

    fn pop(&mut self) {
        if self.modes.len() > 1 {
            let m = self.modes.pop().expect("a mode");
            self.total -= match &m {
                Mode::Code(f) => f.levels(),
                _ => 1,
            };
        }
    }

    fn frame(&mut self) -> &mut Frame {
        match self.modes.last_mut() {
            Some(Mode::Code(f)) => f,
            _ => unreachable!("called in code"),
        }
    }

    /// Changes the levels of the current frame by `f`.
    fn update(&mut self, f: impl FnOnce(&mut Frame)) {
        let frame = self.frame();
        let before = frame.levels();
        f(frame);
        let after = frame.levels();
        self.total = self.total + after - before;
    }

    /// A token starts in code; `is_else` if it is `else`.
    fn token(&mut self, is_else: bool) {
        if self.frame().after_stmt {
            self.update(|f| {
                f.after_stmt = false;
                if !is_else {
                    f.elses = 0;
                }
            });
        }
    }
}

/// The byte offset in `text` at which the estimated nesting passes
/// `SCAN_LIMIT`, or `None`. `tsx` selects JSX.
pub fn too_deep(text: &str, tsx: bool) -> Option<u32> {
    let b = text.as_bytes();
    let n = b.len();
    let at = |i: usize| if i < n { b[i] } else { 0 };
    let mut s = Scan { modes: Vec::new(), total: 0, operand: false };
    s.push(Mode::Code(Frame::new(0)));
    let mut i = 0;
    while i < n {
        if s.total > SCAN_LIMIT {
            return Some(i as u32);
        }
        let c = b[i];
        match s.modes.last().expect("the top level") {
            Mode::Template => match c {
                b'\\' => i += 2,
                b'`' => {
                    s.pop();
                    s.operand = true;
                    i += 1;
                }
                b'$' if at(i + 1) == b'{' => {
                    s.push(Mode::Code(Frame::new(b'}')));
                    s.operand = false;
                    i += 2;
                }
                _ => i += 1,
            },
            Mode::JsxTag => match c {
                b'/' if at(i + 1) == b'>' => {
                    s.pop();
                    s.operand = true;
                    i += 2;
                }
                b'>' => {
                    s.pop();
                    s.push(Mode::JsxChildren);
                    i += 1;
                }
                b'{' => {
                    s.push(Mode::Code(Frame::new(b'}')));
                    s.operand = false;
                    i += 1;
                }
                b'"' | b'\'' => i = skip_string(b, i),
                _ => i += 1,
            },
            Mode::JsxChildren => match c {
                b'{' => {
                    s.push(Mode::Code(Frame::new(b'}')));
                    s.operand = false;
                    i += 1;
                }
                b'<' if at(i + 1) == b'/' => {
                    while i < n && b[i] != b'>' {
                        i += 1;
                    }
                    s.pop();
                    s.operand = true;
                    i += 1;
                }
                b'<' => {
                    s.push(Mode::JsxTag);
                    i += 1;
                }
                _ => i += 1,
            },
            Mode::Code(_) => i = code_token(&mut s, b, i, tsx),
        }
    }
    (s.total > SCAN_LIMIT).then_some(n as u32)
}

/// Scans one token of code at `i`; returns the offset after it.
fn code_token(s: &mut Scan, b: &[u8], i: usize, tsx: bool) -> usize {
    let n = b.len();
    let at = |i: usize| if i < n { b[i] } else { 0 };
    let c = b[i];
    if c.is_ascii_whitespace() {
        return i + 1;
    }
    if c == b'/' && at(i + 1) == b'/' {
        return b[i..].iter().position(|&x| x == b'\n').map_or(n, |p| i + p);
    }
    if c == b'/' && at(i + 1) == b'*' {
        return b[i + 2..].windows(2).position(|w| w == b"*/").map_or(n, |p| i + 2 + p + 2);
    }
    let is_word = |x: u8| x.is_ascii_alphanumeric() || x == b'_' || x == b'$' || x == b'\\' || x >= 0x80;
    if is_word(c) && !c.is_ascii_digit() {
        let mut j = i;
        while j < n && is_word(b[j]) {
            j += 1;
        }
        let word = &b[i..j];
        s.token(word == b"else");
        match word {
            b"typeof" | b"void" | b"delete" | b"await" | b"new" | b"yield" => {
                s.update(|f| f.run += 1);
                s.operand = false;
            }
            b"if" | b"for" | b"while" | b"with" | b"do" => {
                s.update(|f| f.stmts += 1);
                s.operand = false;
            }
            b"else" => {
                s.update(|f| f.elses += 1);
                s.operand = false;
            }
            b"return" | b"throw" | b"case" | b"in" | b"of" | b"instanceof" | b"extends" | b"keyof" | b"as" | b"satisfies"
            | b"default" | b"export" => s.operand = false,
            _ => s.operand = true,
        }
        return j;
    }
    s.token(false);
    let op = |s: &mut Scan, len: usize, f: fn(&mut Frame)| {
        s.update(f);
        s.operand = false;
        i + len
    };
    let plain = |s: &mut Scan, len: usize| {
        s.operand = false;
        i + len
    };
    match c {
        b'0'..=b'9' => {
            let mut j = i;
            while j < n && (is_word(b[j]) || b[j] == b'.') {
                j += 1;
            }
            s.operand = true;
            j
        }
        b'.' if at(i + 1).is_ascii_digit() => {
            let mut j = i + 1;
            while j < n && (is_word(b[j]) || b[j] == b'.') {
                j += 1;
            }
            s.operand = true;
            j
        }
        b'\'' | b'"' => {
            s.operand = true;
            skip_string(b, i)
        }
        b'`' => {
            s.push(Mode::Template);
            i + 1
        }
        b'(' | b'[' | b'{' => {
            let close = match c {
                b'(' => b')',
                b'[' => b']',
                _ => b'}',
            };
            s.push(Mode::Code(Frame::new(close)));
            s.operand = false;
            i + 1
        }
        b')' | b']' | b'}' => {
            if s.frame().close == c {
                s.pop();
                if c == b'}' && matches!(s.modes.last(), Some(Mode::Code(_))) {
                    // A block ends a statement; an `else` may follow.
                    s.update(|f| {
                        f.stmts = 0;
                        f.after_stmt = true;
                    });
                }
            }
            s.operand = true;
            i + 1
        }
        b';' => {
            s.update(|f| {
                f.run = 0;
                f.stmts = 0;
                f.angles = 0;
                f.after_stmt = true;
            });
            s.operand = false;
            i + 1
        }
        b',' => op(s, 1, |f| f.run = 0),
        b'<' if tsx && !s.operand && (at(i + 1).is_ascii_alphabetic() || at(i + 1) == b'>') => {
            s.push(Mode::JsxTag);
            i + 1
        }
        b'<' if at(i + 1) == b'<' || at(i + 1) == b'=' => plain(s, 2),
        b'<' => op(s, 1, |f| f.angles += 1),
        b'>' => {
            let mut k = 1;
            while k < 3 && at(i + k) == b'>' {
                k += 1;
            }
            if at(i + k) == b'=' {
                return plain(s, k + 1);
            }
            s.update(|f| f.angles = f.angles.saturating_sub(k as u32));
            plain(s, k)
        }
        b'=' if at(i + 1) == b'=' => plain(s, if at(i + 2) == b'=' { 3 } else { 2 }),
        b'=' if at(i + 1) == b'>' => op(s, 2, |f| f.run += 1),
        b'=' => op(s, 1, |f| f.run += 1),
        b'+' | b'-' if at(i + 1) == c => {
            if !s.operand {
                s.update(|f| f.run += 1);
            }
            i + 2
        }
        b'+' | b'-' if at(i + 1) == b'=' => op(s, 2, |f| f.run += 1),
        b'+' | b'-' if s.operand => plain(s, 1),
        b'+' | b'-' | b'~' => op(s, 1, |f| f.run += 1),
        b'*' if at(i + 1) == b'*' => op(s, if at(i + 2) == b'=' { 3 } else { 2 }, |f| f.run += 1),
        b'/' if s.operand => plain(s, 1),
        b'/' => {
            s.operand = true;
            skip_regex(b, i)
        }
        b'!' if at(i + 1) == b'=' => plain(s, if at(i + 2) == b'=' { 3 } else { 2 }),
        b'!' => op(s, 1, |f| f.run += 1),
        b'?' if at(i + 1) == b'?' => plain(s, if at(i + 2) == b'=' { 3 } else { 2 }),
        b'?' if at(i + 1) == b'.' && !at(i + 2).is_ascii_digit() => plain(s, 2),
        b'?' => op(s, 1, |f| f.run += 1),
        b'.' if at(i + 1) == b'.' && at(i + 2) == b'.' => op(s, 3, |f| f.run += 1),
        _ => plain(s, 1),
    }
}

/// The offset after the string literal that starts at `i`.
fn skip_string(b: &[u8], i: usize) -> usize {
    let q = b[i];
    let mut j = i + 1;
    while j < b.len() && b[j] != q && b[j] != b'\n' {
        if b[j] == b'\\' {
            j += 1;
        }
        j += 1;
    }
    (j + 1).min(b.len())
}

/// The offset after the regular expression literal that starts at `i`.
fn skip_regex(b: &[u8], i: usize) -> usize {
    let mut j = i + 1;
    let mut class = false;
    while j < b.len() && b[j] != b'\n' {
        match b[j] {
            b'\\' => j += 1,
            b'[' => class = true,
            b']' => class = false,
            b'/' if !class => {
                j += 1;
                while j < b.len() && b[j].is_ascii_alphanumeric() {
                    j += 1;
                }
                return j;
            }
            _ => {}
        }
        j += 1;
    }
    j
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deep(text: &str) -> bool {
        too_deep(text, true).is_some()
    }

    #[test]
    fn chains_and_real_code_pass() {
        let n = 100_000;
        assert!(!deep(&vec!["1"; n].join(" + ")));
        assert!(!deep(&vec!["a"; n].join(" && ")));
        assert!(!deep(&format!("x{}", ".f(1)".repeat(n))));
        assert!(!deep(&vec!["-1"; n].join(", ")));
        assert!(!deep(&vec!["if (a < b) { x = -y; } else { y = !x; }"; n].join("\n")));
        assert!(!deep(&vec!["const a: Map<string, Array<number>> = new Map();"; n].join("\n")));
        assert!(!deep(&vec!["<Text>Don't (stop) here</Text>"; n].join("\n")));
        assert!(!deep(&vec![r"x = /[(]+\//g.test(`a${(b)}c`);"; n].join("\n")));
    }

    #[test]
    fn deep_nesting_stops() {
        let n = SCAN_LIMIT as usize + 10;
        assert!(deep(&"(".repeat(n)));
        assert!(deep(&"[".repeat(n)));
        assert!(deep(&"{".repeat(n)));
        assert!(deep(&format!("{}0", "a ? b : ".repeat(n))));
        assert!(deep(&format!("{}0", "- ".repeat(n))));
        assert!(deep(&format!("{}0", "!".repeat(n))));
        assert!(deep(&format!("{}0", "x = ".repeat(n))));
        assert!(deep(&format!("{}0", "() => ".repeat(n))));
        assert!(deep(&"if (a) ".repeat(n)));
        assert!(deep(&"if (a) {} else ".repeat(n)));
        assert!(deep(&"<Group>".repeat(n)));
        assert!(deep(&"<Group a={1}>text, more; text".repeat(n)));
        assert!(deep(&"Array<".repeat(n)));
        assert!(deep(&"`${".repeat(n)));
    }
}
