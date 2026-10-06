//! Diagnostics (SPEC.md §5.2).
//!
//! Every rejected feature has a stable code. The text format is the same as
//! the `tsc` format (`file(line,col): error PL1001: message`), so editors and
//! terminals link it. `--json` gives the same data as JSON.

use std::fmt::Write as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct FileId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub file: FileId,
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(file: FileId, start: u32, end: u32) -> Self {
        Self { file, start, end }
    }
}

/// The diagnostic codes. Keep them stable: tests and docs refer to them.
pub mod code {
    // 1xxx: modules and the closed world.
    pub const SYNTAX: &str = "PL1000";
    pub const BARE_IMPORT: &str = "PL1001";
    pub const MODULE_NOT_FOUND: &str = "PL1002";
    pub const IMPORT_CYCLE: &str = "PL1003";
    pub const UNKNOWN_EXPORT: &str = "PL1004";
    pub const UNKNOWN_STD_MODULE: &str = "PL1005";
    pub const NO_APP: &str = "PL1006";
    /// A `plinth:*` host API call needs a capability that `plinth.toml`
    /// does not declare (SPEC.md §11).
    pub const CAPABILITY_UNDECLARED: &str = "PL1007";
    // 2xxx: rejected language features (SPEC.md §4.4).
    pub const UNSUPPORTED: &str = "PL2000";
    pub const ANY: &str = "PL2001";
    pub const LOOSE_EQUALITY: &str = "PL2002";
    pub const CLASS: &str = "PL2003";
    pub const THIS: &str = "PL2004";
    pub const NON_NULL_ASSERTION: &str = "PL2005";
    pub const TYPE_ASSERTION: &str = "PL2006";
    pub const FOR_IN: &str = "PL2007";
    pub const DELETE: &str = "PL2008";
    pub const ASYNC: &str = "PL2009";
    pub const TRY: &str = "PL2010";
    pub const COMPUTED_ACCESS: &str = "PL2011";
    pub const ADVANCED_TYPE: &str = "PL2012";
    pub const NAMESPACE: &str = "PL2013";
    pub const VAR: &str = "PL2014";
    pub const GENERIC_USER: &str = "PL2015";
    pub const GETTER_SETTER: &str = "PL2016";
    pub const LABEL: &str = "PL2017";
    pub const REGEX: &str = "PL2018";
    pub const BIGINT: &str = "PL2019";
    pub const SIGNAL_NOT_REACTIVE: &str = "PL2020";
    /// A class method is referenced without calling it (SPEC.md §4.2):
    /// methods are static-dispatch functions, so a bare `c.m` has no value.
    pub const UNBOUND_METHOD: &str = "PL2021";
    /// `extends` names something that is not a class, or the hierarchy has
    /// a cycle (SPEC.md §4.2 v1).
    pub const EXTENDS: &str = "PL2022";
    /// `super(...)`/`super.m(...)` used outside a subclass constructor or
    /// method, missing, or not the constructor's first statement.
    pub const SUPER: &str = "PL2023";
    /// A method override's signature is not compatible with the base
    /// class's method.
    pub const OVERRIDE: &str = "PL2024";
    // 3xxx: types.
    pub const TYPE_MISMATCH: &str = "PL3001";
    pub const UNKNOWN_NAME: &str = "PL3002";
    pub const UNKNOWN_TYPE: &str = "PL3003";
    pub const NO_PROPERTY: &str = "PL3004";
    pub const NOT_CALLABLE: &str = "PL3005";
    pub const ARG_COUNT: &str = "PL3006";
    pub const CANNOT_INFER: &str = "PL3007";
    pub const ASSIGN_CONST: &str = "PL3008";
    pub const MISSING_FIELD: &str = "PL3009";
    pub const NULLABLE: &str = "PL3010";
    /// Reserved for a struct-layout error; no check emits it today (there
    /// is no golden test for it — see `tests/golden.rs`). Kept stable in
    /// case a future layout check needs it.
    pub const STRUCT_LAYOUT: &str = "PL3011";
    pub const MISSING_RETURN: &str = "PL3012";
    pub const DUPLICATE: &str = "PL3013";
    // 4xxx: JSX and the UI API.
    pub const UNKNOWN_CONTROL: &str = "PL4001";
    pub const UNKNOWN_PROP: &str = "PL4002";
    pub const MISSING_PROP: &str = "PL4003";
    pub const BAD_CHILD: &str = "PL4004";
    pub const BAD_BINDING: &str = "PL4005";
    pub const BAD_APP: &str = "PL4006";
    pub const BAD_NAVIGATE: &str = "PL4007";
    pub const BAD_ASSET: &str = "PL4008";
    pub const EMPTY_ALT: &str = "PL4009";
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
    pub span: Span,
    pub help: Option<String>,
}

impl Diagnostic {
    pub fn error(code: &'static str, span: Span, message: impl Into<String>) -> Self {
        Self { severity: Severity::Error, code, message: message.into(), span, help: None }
    }

    pub fn warning(code: &'static str, span: Span, message: impl Into<String>) -> Self {
        Self { severity: Severity::Warning, code, message: message.into(), span, help: None }
    }

    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
}

/// The source files of one compilation.
#[derive(Default)]
pub struct Sources {
    files: Vec<(String, String)>,
}

impl Sources {
    pub fn add(&mut self, path: String, text: String) -> FileId {
        self.files.push((path, text));
        FileId(self.files.len() as u32 - 1)
    }

    pub fn path(&self, f: FileId) -> &str {
        &self.files[f.0 as usize].0
    }

    pub fn text(&self, f: FileId) -> &str {
        &self.files[f.0 as usize].1
    }

    /// 1-based line and column (in UTF-16 units, as tsc reports them).
    pub fn line_col(&self, span: Span) -> (usize, usize) {
        let text = self.text(span.file);
        let offset = (span.start as usize).min(text.len());
        let before = &text[..offset];
        let line = before.matches('\n').count() + 1;
        let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
        let col = before[line_start..].encode_utf16().count() + 1;
        (line, col)
    }

    /// The tsc-style text form, with the source line and a caret.
    pub fn render(&self, d: &Diagnostic) -> String {
        let (line, col) = self.line_col(d.span);
        let sev = match d.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        let mut out = format!("{}({line},{col}): {sev} {}: {}", self.path(d.span.file), d.code, d.message);
        let text = self.text(d.span.file);
        if let Some(src) = text.lines().nth(line - 1) {
            let width = ((d.span.end.saturating_sub(d.span.start)) as usize).clamp(1, src.len().saturating_sub(col - 1).max(1));
            let _ = write!(out, "\n  {src}\n  {}{}", " ".repeat(col - 1), "^".repeat(width));
        }
        if let Some(help) = &d.help {
            let _ = write!(out, "\n  help: {help}");
        }
        out
    }

    pub fn to_json(&self, diags: &[Diagnostic]) -> String {
        let mut out = String::from("[");
        for (i, d) in diags.iter().enumerate() {
            let (line, col) = self.line_col(d.span);
            if i > 0 {
                out.push(',');
            }
            let _ = write!(
                out,
                "{{\"file\":{},\"line\":{line},\"column\":{col},\"start\":{},\"end\":{},\"severity\":\"{}\",\"code\":\"{}\",\"message\":{},\"help\":{}}}",
                json_str(self.path(d.span.file)),
                d.span.start,
                d.span.end,
                if d.severity == Severity::Error { "error" } else { "warning" },
                d.code,
                json_str(&d.message),
                d.help.as_deref().map(json_str).unwrap_or_else(|| "null".into()),
            );
        }
        out.push(']');
        out
    }
}

fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
