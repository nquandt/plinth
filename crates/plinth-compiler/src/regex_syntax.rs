//! Compile-time validation of a `/pattern/flags` regex literal (or a
//! `new RegExp("literal-looking string")` argument that happens to be a
//! string literal — not done yet, only `/.../ ` literals use this).
//!
//! This is a deliberately small, parse-only subset of the real parser in
//! `plinth-rt/src/regex.rs` (the Pike-VM matcher that actually runs the
//! pattern at run time). It is a separate copy, not a shared dependency,
//! because `plinth-rt` is a guest-only crate that only ever builds for
//! `wasm32-unknown-unknown` (see the workspace `default-members` comment);
//! `plinth-compiler` runs on the host and must not pull it into that graph.
//! Keep the accepted syntax here in sync with `plinth-rt/src/regex.rs`'s
//! module docs: literal chars, `.`, `[...]` classes (ranges, `^`
//! negation), escapes `\d \w \s \D \W \S \b \B \n \t \r \f \v \\ \/ \.` and
//! other identity escapes, anchors `^ $`, groups `( )`, non-capturing
//! `(?: )`, named groups `(?<name> )`, alternation `|`, and quantifiers
//! `* + ? {n} {n,} {n,m}` with their lazy forms. Lookaround and
//! backreferences are rejected. Flags: only `g i m s`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    UnterminatedGroup,
    UnterminatedClass,
    InvalidQuantifier,
    QuantifierWithoutTarget,
    TrailingEscape,
    UnmatchedCloseParen,
    Unsupported,
    BadFlag(char),
}

impl ErrorKind {
    pub fn message(self) -> String {
        match self {
            ErrorKind::UnterminatedGroup => "an unterminated group: a `(` has no matching `)`".to_string(),
            ErrorKind::UnterminatedClass => "an unterminated character class: a `[` has no matching `]`".to_string(),
            ErrorKind::InvalidQuantifier => "an invalid quantifier (e.g. `{2,1}` with a minimum above the maximum)".to_string(),
            ErrorKind::QuantifierWithoutTarget => "a quantifier (`* + ? {{..}}`) with nothing before it to repeat".to_string(),
            ErrorKind::TrailingEscape => "a trailing backslash with no character after it".to_string(),
            ErrorKind::UnmatchedCloseParen => "a `)` with no matching `(`".to_string(),
            ErrorKind::Unsupported => {
                "lookaround (`(?=` `(?!` `(?<=` `(?<!`) and backreferences (a backslash followed by a digit) are not supported yet".to_string()
            }
            ErrorKind::BadFlag(c) => format!("the flag `{c}` is not supported (only `g`, `i`, `m` and `s` are)"),
        }
    }
}

/// Validates a pattern and flag string, same syntax as `plinth-rt`'s
/// `regex::parse` would accept. Returns the first error found.
pub fn validate(pattern: &str, flags: &str) -> Result<(), ErrorKind> {
    for c in flags.chars() {
        if !matches!(c, 'g' | 'i' | 'm' | 's') {
            return Err(ErrorKind::BadFlag(c));
        }
    }
    let chars: Vec<char> = pattern.chars().collect();
    let mut p = Parser { chars: &chars, pos: 0 };
    p.parse_alt()?;
    if p.pos != p.chars.len() {
        return Err(ErrorKind::UnmatchedCloseParen);
    }
    Ok(())
}

type PResult<T> = Result<T, ErrorKind>;

struct Parser<'a> {
    chars: &'a [char],
    pos: usize,
}

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

    fn parse_alt(&mut self) -> PResult<()> {
        self.parse_concat()?;
        while self.eat('|') {
            self.parse_concat()?;
        }
        Ok(())
    }

    fn parse_concat(&mut self) -> PResult<()> {
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            self.parse_repeat()?;
        }
        Ok(())
    }

    fn parse_repeat(&mut self) -> PResult<()> {
        let had_atom = self.parse_atom()?;
        self.parse_quantifier(had_atom)
    }

    fn parse_quantifier(&mut self, had_atom: bool) -> PResult<()> {
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
                    return Ok(());
                }
            }
            _ => return Ok(()),
        };
        if let Some(mx) = max {
            if min > mx {
                return Err(ErrorKind::InvalidQuantifier);
            }
        }
        if !had_atom {
            return Err(ErrorKind::QuantifierWithoutTarget);
        }
        self.eat('?');
        Ok(())
    }

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

    /// Returns whether an atom was actually consumed (an empty match at
    /// end-of-pattern consumes nothing, and a quantifier right after that
    /// is an error).
    fn parse_atom(&mut self) -> PResult<bool> {
        match self.peek() {
            None => Ok(false),
            Some('(') => {
                self.parse_group()?;
                Ok(true)
            }
            Some('[') => {
                self.parse_class()?;
                Ok(true)
            }
            Some('.') => {
                self.pos += 1;
                Ok(true)
            }
            Some('^') => {
                self.pos += 1;
                Ok(true)
            }
            Some('$') => {
                self.pos += 1;
                Ok(true)
            }
            Some('\\') => {
                self.parse_escape()?;
                Ok(true)
            }
            Some('*') | Some('+') | Some('?') => Err(ErrorKind::QuantifierWithoutTarget),
            Some(_) => {
                self.pos += 1;
                Ok(true)
            }
        }
    }

    fn parse_group(&mut self) -> PResult<()> {
        self.pos += 1; // '('
        if self.eat('?') {
            match self.peek() {
                Some(':') => {
                    self.pos += 1;
                    self.parse_alt()?;
                    if !self.eat(')') {
                        return Err(ErrorKind::UnterminatedGroup);
                    }
                    Ok(())
                }
                Some('=') | Some('!') => Err(ErrorKind::Unsupported),
                Some('<') => {
                    if matches!(self.peek_at(1), Some('=') | Some('!')) {
                        return Err(ErrorKind::Unsupported);
                    }
                    self.pos += 1; // '<'
                    while matches!(self.peek(), Some(c) if c != '>') {
                        self.pos += 1;
                    }
                    if !self.eat('>') {
                        return Err(ErrorKind::UnterminatedGroup);
                    }
                    self.parse_alt()?;
                    if !self.eat(')') {
                        return Err(ErrorKind::UnterminatedGroup);
                    }
                    Ok(())
                }
                _ => Err(ErrorKind::UnterminatedGroup),
            }
        } else {
            self.parse_alt()?;
            if !self.eat(')') {
                return Err(ErrorKind::UnterminatedGroup);
            }
            Ok(())
        }
    }

    fn parse_class(&mut self) -> PResult<()> {
        self.pos += 1; // '['
        self.eat('^');
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
            if let ClassAtom::Char(lo_c) = lo {
                if self.peek() == Some('-') && self.peek_at(1) != Some(']') && self.peek_at(1).is_some() {
                    self.pos += 1; // '-'
                    match self.parse_class_atom()? {
                        ClassAtom::Char(hi_c) => {
                            if hi_c < lo_c {
                                return Err(ErrorKind::InvalidQuantifier);
                            }
                        }
                        ClassAtom::Esc => return Err(ErrorKind::InvalidQuantifier),
                    }
                }
            }
        }
        Ok(())
    }

    fn parse_class_atom(&mut self) -> PResult<ClassAtom> {
        if self.peek() == Some('\\') {
            self.pos += 1;
            let c = self.bump().ok_or(ErrorKind::TrailingEscape)?;
            Ok(match c {
                'd' | 'D' | 'w' | 'W' | 's' | 'S' => ClassAtom::Esc,
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

    fn parse_escape(&mut self) -> PResult<()> {
        self.pos += 1; // '\\'
        let c = self.bump().ok_or(ErrorKind::TrailingEscape)?;
        match c {
            'd' | 'D' | 'w' | 'W' | 's' | 'S' | 'b' | 'B' | 'n' | 't' | 'r' | 'f' | 'v' | '0' => Ok(()),
            other if other.is_ascii_digit() => Err(ErrorKind::Unsupported),
            _ => Ok(()),
        }
    }
}

enum ClassAtom {
    Char(char),
    Esc,
}

fn digits_to_u32(chars: &[char]) -> u32 {
    let mut v: u32 = 0;
    for c in chars {
        v = v.saturating_mul(10).saturating_add((*c as u32) - ('0' as u32));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_supported_syntax() {
        assert!(validate("a.b[c-z]+(x|y)(?:z)(?<n>w)\\d\\w\\s^$", "gims").is_ok());
        assert!(validate("a{2,3}?", "").is_ok());
    }

    #[test]
    fn rejects_bad_syntax() {
        assert_eq!(validate("(", "").unwrap_err(), ErrorKind::UnterminatedGroup);
        assert_eq!(validate("[", "").unwrap_err(), ErrorKind::UnterminatedClass);
        assert_eq!(validate("a{2,1}", "").unwrap_err(), ErrorKind::InvalidQuantifier);
        assert_eq!(validate("*a", "").unwrap_err(), ErrorKind::QuantifierWithoutTarget);
        assert_eq!(validate(")", "").unwrap_err(), ErrorKind::UnmatchedCloseParen);
    }

    #[test]
    fn rejects_unsupported() {
        assert_eq!(validate("(?=a)", "").unwrap_err(), ErrorKind::Unsupported);
        assert_eq!(validate("(?!a)", "").unwrap_err(), ErrorKind::Unsupported);
        assert_eq!(validate("(?<=a)", "").unwrap_err(), ErrorKind::Unsupported);
        assert_eq!(validate("(?<!a)", "").unwrap_err(), ErrorKind::Unsupported);
        assert_eq!(validate("(a)\\1", "").unwrap_err(), ErrorKind::Unsupported);
    }

    #[test]
    fn rejects_bad_flags() {
        assert_eq!(validate("a", "y").unwrap_err(), ErrorKind::BadFlag('y'));
        assert_eq!(validate("a", "u").unwrap_err(), ErrorKind::BadFlag('u'));
    }
}
