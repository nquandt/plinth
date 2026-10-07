//! The `plinth:*` modules (SPEC.md §4.7): names, calls and the `app()`
//! entry. `std/*.d.ts` declares the same names for the editor; the test
//! `std_typings_match` keeps them in sync.

use super::{Binding, Checker, StdFn, StdModule, StdObj};
use crate::ast::{self, Expr, ExprKind, ObjProp};
use crate::controls;
use crate::diag::{Span, code};
use crate::tir::*;
use crate::types::Type;

/// `GET` (SPEC.md §8.5): `fetch`'s default method when `options` is `null`
/// or omits `method`.
const DEFAULT_METHOD: &str = "GET";

/// The value and type names that each std module exports.
pub const UI_NAMES: &[&str] = &[
    "signal", "computed", "effect", "app", "navigate", "Signal", "Computed", "Accent", "IconName", "ScreenDef", "AppConfig",
    "App", "Tone", "Align", "Group", "Screen", "Section", "Text", "Heading", "Button", "TextField", "Toggle", "List", "Row", "Empty",
    "Checkbox", "TextArea", "Slider", "NumberField", "Picker", "Progress", "Badge",
    // UI API 1.2
    "Tabs", "Sheet", "Dialog", "Menu", "Grid", "Action",
    // UI API 1.3
    "Image", "Aspect",
    // UI API 1.4
    "Icon", "DatePicker", "Box", "Span", "Pressable", "Scroll", "Canvas", "Shape", "rect", "circle", "line", "canvasText",
    // UI API 1.5
    "Chart", "ChartPoint", "ChartSeriesDef",
];
pub const CORE_NAMES: &[&str] = &["Math", "parseNumber", "toString", "console", "int", "int", "JSON"];
pub const TIME_NAMES: &[&str] = &[
    "now",
    "monotonicNow",
    "setTimeout",
    "setInterval",
    "clearTimeout",
    "clearInterval",
    // Date/time additions (docs/GAPS.md gap #5).
    "timezoneOffset",
    "dateParts",
    "makeDate",
    "formatDate",
    "toISOString",
    "parseDate",
    "DateParts",
];
pub const STORE_NAMES: &[&str] = &["kv"];
pub const CLIPBOARD_NAMES: &[&str] = &["writeText", "readText", "lastError"];
pub const DIALOG_NAMES: &[&str] = &["alert", "confirm", "prompt"];
pub const NET_NAMES: &[&str] = &["fetch", "Response", "FetchOptions"];
pub const FILES_NAMES: &[&str] = &["read", "write", "list", "stat", "remove", "FileEntry"];
pub const HUB_NAMES: &[&str] = &[
    "listApps",
    "launch",
    "setGrant",
    "block",
    "unblock",
    "lastError",
    "listGroups",
    "createGroup",
    "setGroup",
    "remove",
    "search",
    "install",
    "appInfo",
    "pin",
    "blockPublisher",
    "unblockPublisher",
    "checkUpdates",
    "update",
];

/// The capability names, from the shared map (`plinth-link`'s
/// `capabilities` module, `docs/HUB.md` §12.3) rather than a duplicated
/// list here.
pub use plinth_link::capabilities::{
    CLIPBOARD_READ as CAP_CLIPBOARD_READ, CLIPBOARD_WRITE as CAP_CLIPBOARD_WRITE, FILES_PRIVATE as CAP_FILES_PRIVATE, HUB_MANAGE as CAP_HUB_MANAGE, STORE_KV as CAP_STORE_KV,
};

pub fn lookup(m: StdModule, name: &str) -> Option<Binding> {
    match m {
        StdModule::Time => Some(match name {
            "now" => Binding::Std(StdFn::TimeNow),
            "monotonicNow" => Binding::Std(StdFn::TimeMonotonicNow),
            "setTimeout" => Binding::Std(StdFn::SetTimeout),
            "setInterval" => Binding::Std(StdFn::SetInterval),
            "clearTimeout" | "clearInterval" => Binding::Std(StdFn::ClearTimer),
            "timezoneOffset" => Binding::Std(StdFn::TimezoneOffset),
            "dateParts" => Binding::Std(StdFn::DateParts),
            "makeDate" => Binding::Std(StdFn::MakeDate),
            "formatDate" => Binding::Std(StdFn::FormatDate),
            "toISOString" => Binding::Std(StdFn::ToIsoString),
            "parseDate" => Binding::Std(StdFn::ParseDate),
            // `DateParts` is shaped structurally, like `net.fetch`'s
            // `Response` (placeholder so `tsc`/`std_typings_match` see the
            // name; the editor's real type comes from `time.d.ts`).
            "DateParts" => Binding::Type(Type::Error),
            _ => return None,
        }),
        StdModule::Store => Some(match name {
            "kv" => Binding::StdObj(StdObj::Kv),
            _ => return None,
        }),
        StdModule::Clipboard => Some(match name {
            "writeText" => Binding::Std(StdFn::ClipboardWriteText),
            "readText" => Binding::Std(StdFn::ClipboardReadText),
            "lastError" => Binding::Std(StdFn::ClipboardLastError),
            _ => return None,
        }),
        StdModule::Dialog => Some(match name {
            "alert" => Binding::Std(StdFn::DialogAlert),
            "confirm" => Binding::Std(StdFn::DialogConfirm),
            "prompt" => Binding::Std(StdFn::DialogPrompt),
            _ => return None,
        }),
        StdModule::Net => Some(match name {
            "fetch" => Binding::Std(StdFn::NetFetch),
            // `Response` is shaped structurally (the `done` callback's
            // parameter type is inferred, like `confirm`'s `ok: boolean`):
            // this placeholder exists only so `tsc` and `std_typings_match`
            // see the name declared; the editor's own types come from
            // `net.d.ts` directly.
            "Response" | "FetchOptions" => Binding::Type(Type::Error),
            _ => return None,
        }),
        StdModule::Files => Some(match name {
            "read" => Binding::Std(StdFn::FilesRead),
            "write" => Binding::Std(StdFn::FilesWrite),
            "list" => Binding::Std(StdFn::FilesList),
            "stat" => Binding::Std(StdFn::FilesStat),
            "remove" => Binding::Std(StdFn::FilesRemove),
            // Shaped structurally, like `net`'s `Response` (the editor's
            // type comes from `files.d.ts`).
            "FileEntry" => Binding::Type(Type::Error),
            _ => return None,
        }),
        StdModule::Hub => Some(match name {
            "listApps" => Binding::Std(StdFn::HubListApps),
            "launch" => Binding::Std(StdFn::HubLaunch),
            "setGrant" => Binding::Std(StdFn::HubSetGrant),
            "block" => Binding::Std(StdFn::HubBlock),
            "unblock" => Binding::Std(StdFn::HubUnblock),
            "lastError" => Binding::Std(StdFn::HubLastError),
            "listGroups" => Binding::Std(StdFn::HubListGroups),
            "createGroup" => Binding::Std(StdFn::HubCreateGroup),
            "setGroup" => Binding::Std(StdFn::HubSetGroup),
            "remove" => Binding::Std(StdFn::HubRemove),
            "search" => Binding::Std(StdFn::HubSearch),
            "install" => Binding::Std(StdFn::HubInstall),
            "appInfo" => Binding::Std(StdFn::HubAppInfo),
            "pin" => Binding::Std(StdFn::HubPin),
            "blockPublisher" => Binding::Std(StdFn::HubBlockPublisher),
            "unblockPublisher" => Binding::Std(StdFn::HubUnblockPublisher),
            "checkUpdates" => Binding::Std(StdFn::HubCheckUpdates),
            "update" => Binding::Std(StdFn::HubUpdate),
            _ => return None,
        }),
        StdModule::Ui => Some(match name {
            "signal" => Binding::Std(StdFn::Signal),
            "computed" => Binding::Std(StdFn::Computed),
            "effect" => Binding::Std(StdFn::Effect),
            "app" => Binding::Std(StdFn::App),
            "navigate" => Binding::StdObj(StdObj::Navigate),
            "Signal" => Binding::Type(Type::Signal(Box::new(Type::Error))),
            "Computed" => Binding::Type(Type::Computed(Box::new(Type::Error))),
            "Accent" => Binding::Type(Type::str_lits(controls::ACCENTS.iter().map(|s| s.to_string()).collect())),
            "IconName" => Binding::Type(Type::str_lits(controls::ICONS.iter().map(|s| s.to_string()).collect())),
            "Align" => Binding::Type(Type::str_lits(["start", "center", "end"].iter().map(|s| s.to_string()).collect())),
            "Tone" => Binding::Type(Type::str_lits(["default", "muted", "danger", "success"].iter().map(|s| s.to_string()).collect())),
            "Aspect" => Binding::Type(Type::str_lits(["square", "wide", "tall"].iter().map(|s| s.to_string()).collect())),
            "App" => Binding::Type(Type::App),
            // UI API 1.10: Canvas shapes. A `Shape` is a string at run time.
            "rect" => Binding::Std(StdFn::ShapeRect),
            "circle" => Binding::Std(StdFn::ShapeCircle),
            "line" => Binding::Std(StdFn::ShapeLine),
            "canvasText" => Binding::Std(StdFn::ShapeText),
            "Shape" => Binding::Type(Type::String),
            // Config shapes exist for the editor only.
            "ScreenDef" | "AppConfig" => Binding::Type(Type::Error),
            _ => Binding::Control(controls::by_name(name)?.kind),
        }),
        StdModule::Core => Some(match name {
            "Math" => Binding::StdObj(StdObj::Math),
            "console" => Binding::StdObj(StdObj::Console),
            "parseNumber" => Binding::Std(StdFn::ParseNumber),
            "toString" => Binding::Std(StdFn::ToString),
            "int" => Binding::Std(StdFn::Int),
            "JSON" => Binding::StdObj(StdObj::Json),
            _ => return None,
        }),
    }
}

impl Checker<'_> {
    pub(super) fn std_call(
        &mut self,
        f: StdFn,
        type_args: &[ast::TypeAnn],
        args: &[Expr],
        span: Span,
        _expected: Option<&Type>,
    ) -> TExpr {
        let one_arg = |c: &mut Self, what: &str| -> bool {
            if args.len() != 1 {
                c.err(code::ARG_COUNT, span, format!("`{what}` takes one argument"));
                false
            } else {
                true
            }
        };
        match f {
            StdFn::Signal => {
                if type_args.len() > 1 {
                    self.err(code::ARG_COUNT, span, "`signal` takes one type argument");
                }
                let declared = type_args.first().map(|t| self.resolve_type(t));
                if !one_arg(self, "signal") {
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let init = match &declared {
                    Some(t) => {
                        let te = self.expr_with(&args[0], t);
                        self.coerce(te, t)
                    }
                    None => self.expr(&args[0], None),
                };
                let ty = match declared {
                    Some(t) => t,
                    None => match &init.ty {
                        Type::Null => {
                            self.err_help(code::CANNOT_INFER, span, "the type of `signal(null)` is unknown", "write `signal<T | null>(null)`");
                            Type::Error
                        }
                        t => self.widen(t.clone()),
                    },
                };
                if let Type::Signal(_) | Type::Computed(_) | Type::Void = ty {
                    self.err(code::TYPE_MISMATCH, span, "a signal cannot hold this value");
                }
                TExpr::new(TExprKind::SignalNew(Box::new(init)), Type::Signal(Box::new(ty)), span)
            }
            StdFn::Computed => {
                if !one_arg(self, "computed") {
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let ret = type_args.first().map(|t| self.resolve_type(t));
                self.pending_reactive = true;
                let (f, _) = self.callback(&args[0], &[], ret.clone());
                let inner = match (&ret, &f.ty) {
                    (Some(r), _) => r.clone(),
                    (None, Type::Func(ft)) => self.widen(ft.ret.clone()),
                    _ => Type::Error,
                };
                if inner == Type::Void {
                    self.err(code::TYPE_MISMATCH, span, "a computed value must return a value; use `effect` for side effects");
                }
                TExpr::new(TExprKind::ComputedNew(Box::new(f)), Type::Computed(Box::new(inner)), span)
            }
            StdFn::Effect => {
                if !one_arg(self, "effect") {
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                self.pending_reactive = true;
                let (f, _) = self.callback(&args[0], &[], Some(Type::Void));
                TExpr::new(TExprKind::EffectNew(Box::new(f)), Type::Void, span)
            }
            StdFn::App => {
                self.err_help(code::BAD_APP, span, "`app()` must be the default export of app/main.tsx", "write `export default app({...})`");
                TExpr::new(TExprKind::Null, Type::Error, span)
            }
            StdFn::Navigate => {
                if !one_arg(self, "navigate") {
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                match &args[0].kind {
                    ExprKind::Str(name) => {
                        self.navigations.push((name.clone(), args[0].span, true));
                        TExpr::new(TExprKind::Navigate(name.clone()), Type::Void, span)
                    }
                    _ => {
                        self.err_help(code::BAD_NAVIGATE, args[0].span, "`navigate` takes a screen name literal", "write `navigate(\"settings\")`");
                        TExpr::new(TExprKind::Null, Type::Error, span)
                    }
                }
            }
            StdFn::ParseNumber => {
                if !one_arg(self, "parseNumber") {
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let te = self.expr(&args[0], Some(&Type::String));
                let te = self.coerce(te, &Type::String);
                TExpr::new(TExprKind::Rt("str_to_f64", vec![te]), Type::Number, span)
            }
            StdFn::ToString => {
                if !one_arg(self, "toString") {
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let te = self.expr(&args[0], None);
                self.to_str(te)
            }
            StdFn::ShapeRect | StdFn::ShapeCircle | StdFn::ShapeLine | StdFn::ShapeText => self.shape_call(f, args, span),
            StdFn::TimeNow => {
                if !args.is_empty() {
                    self.err(code::ARG_COUNT, span, "`now` takes no arguments");
                }
                TExpr::new(TExprKind::Rt("time_now", Vec::new()), Type::Number, span)
            }
            StdFn::TimeMonotonicNow => {
                if !args.is_empty() {
                    self.err(code::ARG_COUNT, span, "`monotonicNow` takes no arguments");
                }
                TExpr::new(TExprKind::Rt("time_monotonic_now", Vec::new()), Type::Number, span)
            }
            StdFn::SetTimeout | StdFn::SetInterval => {
                let what = if f == StdFn::SetTimeout { "setTimeout" } else { "setInterval" };
                if args.len() != 2 {
                    self.err(code::ARG_COUNT, span, format!("`{what}` takes a callback and a delay in milliseconds"));
                    return TExpr::new(TExprKind::Num(0.0), Type::Number, span);
                }
                let (cb, _) = self.callback(&args[0], &[], Some(Type::Void));
                let ms = self.expr_with(&args[1], &Type::Number);
                let ms = self.coerce(ms, &Type::Number);
                TExpr::new(TExprKind::TimerNew(Box::new(ms), f == StdFn::SetInterval, Box::new(cb)), Type::Number, span)
            }
            StdFn::ClearTimer => {
                if !one_arg(self, "clearTimeout/clearInterval") {
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let id = self.expr_with(&args[0], &Type::Number);
                let id = self.coerce(id, &Type::Number);
                TExpr::new(TExprKind::Rt("clear_timer", vec![id]), Type::Void, span)
            }
            StdFn::TimezoneOffset => {
                if !one_arg(self, "timezoneOffset") {
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let ms = self.expr_with(&args[0], &Type::Number);
                let ms = self.coerce(ms, &Type::Number);
                TExpr::new(TExprKind::Rt("tz_offset_minutes", vec![ms]), Type::Number, span)
            }
            StdFn::DateParts => {
                if args.is_empty() || args.len() > 2 {
                    self.err(code::ARG_COUNT, span, "`dateParts` takes a timestamp and an optional `utc` flag");
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let ms = self.expr_with(&args[0], &Type::Number);
                let ms = self.coerce(ms, &Type::Number);
                let utc = match args.get(1) {
                    Some(a) => {
                        let u = self.expr_with(a, &Type::Bool);
                        self.coerce(u, &Type::Bool)
                    }
                    None => TExpr::new(TExprKind::Bool(false), Type::Bool, span),
                };
                let sid = self.date_parts_struct();
                // `ms` and `utc` are each used 8 times (once per field);
                // bind them to locals first so the argument expressions
                // are evaluated exactly once, as JS semantics require.
                let ms_v = self.temp(Type::Number);
                let utc_v = self.temp(Type::Bool);
                let ms_read = || TExpr::new(TExprKind::Var(ms_v), Type::Number, span);
                let utc_read = || TExpr::new(TExprKind::Var(utc_v), Type::Bool, span);
                let mk = |idx: i32| {
                    TExpr::new(
                        TExprKind::Rt("date_field", vec![ms_read(), utc_read(), TExpr::new(TExprKind::Num(idx as f64), Type::Int, span)]),
                        Type::Number,
                        span,
                    )
                };
                let lit = TExpr::new(TExprKind::StructLit(sid, (0..8).map(mk).collect()), Type::Struct(sid), span);
                TExpr::new(TExprKind::Block(vec![TStmt::Let(ms_v, Some(ms)), TStmt::Let(utc_v, Some(utc))], Box::new(lit)), Type::Struct(sid), span)
            }
            StdFn::MakeDate => {
                if args.len() < 3 || args.len() > 6 {
                    self.err(code::ARG_COUNT, span, "`makeDate` takes year, month, day, and optional hour, minute, second");
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let num = |c: &mut Self, e: &Expr| {
                    let te = c.expr_with(e, &Type::Number);
                    c.coerce(te, &Type::Number)
                };
                let zero = || TExpr::new(TExprKind::Num(0.0), Type::Number, span);
                let year = num(self, &args[0]);
                let month = num(self, &args[1]);
                let day = num(self, &args[2]);
                let hour = args.get(3).map(|a| num(self, a)).unwrap_or_else(zero);
                let minute = args.get(4).map(|a| num(self, a)).unwrap_or_else(zero);
                let second = args.get(5).map(|a| num(self, a)).unwrap_or_else(zero);
                TExpr::new(TExprKind::Rt("make_date", vec![year, month, day, hour, minute, second]), Type::Number, span)
            }
            StdFn::FormatDate => {
                if args.len() < 2 || args.len() > 3 {
                    self.err(code::ARG_COUNT, span, "`formatDate` takes a timestamp, a pattern, and an optional `utc` flag");
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let ms = self.expr_with(&args[0], &Type::Number);
                let ms = self.coerce(ms, &Type::Number);
                let pattern = self.expr_with(&args[1], &Type::String);
                let pattern = self.coerce(pattern, &Type::String);
                let utc = match args.get(2) {
                    Some(a) => {
                        let u = self.expr_with(a, &Type::Bool);
                        self.coerce(u, &Type::Bool)
                    }
                    None => TExpr::new(TExprKind::Bool(false), Type::Bool, span),
                };
                TExpr::new(TExprKind::Rt("format_date", vec![ms, pattern, utc]), Type::String, span)
            }
            StdFn::ToIsoString => {
                if !one_arg(self, "toISOString") {
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let ms = self.expr_with(&args[0], &Type::Number);
                let ms = self.coerce(ms, &Type::Number);
                let pattern = TExpr::new(TExprKind::Str("YYYY-MM-DDTHH:mm:ss.SSS".into()), Type::String, span);
                let utc = TExpr::new(TExprKind::Bool(true), Type::Bool, span);
                let formatted = TExpr::new(TExprKind::Rt("format_date", vec![ms, pattern, utc]), Type::String, span);
                let z = TExpr::new(TExprKind::Str("Z".into()), Type::String, span);
                TExpr::new(TExprKind::Concat(Box::new(formatted), Box::new(z)), Type::String, span)
            }
            StdFn::ParseDate => {
                if !one_arg(self, "parseDate") {
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let text = self.expr_with(&args[0], &Type::String);
                let text = self.coerce(text, &Type::String);
                TExpr::new(TExprKind::Rt("parse_date", vec![text]), Type::Number.nullable(), span)
            }
            StdFn::ClipboardWriteText => {
                self.require_capability(CAP_CLIPBOARD_WRITE, span);
                if !one_arg(self, "writeText") {
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let text = self.expr_with(&args[0], &Type::String);
                let text = self.coerce(text, &Type::String);
                TExpr::new(TExprKind::Rt("clipboard_write_text", vec![text]), Type::Void, span)
            }
            StdFn::ClipboardReadText => {
                self.require_capability(CAP_CLIPBOARD_READ, span);
                if !args.is_empty() {
                    self.err(code::ARG_COUNT, span, "`readText` takes no arguments");
                }
                TExpr::new(TExprKind::Rt("clipboard_read_text", Vec::new()), Type::String.nullable(), span)
            }
            StdFn::Int => {
                if !one_arg(self, "int") {
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let te = self.expr(&args[0], Some(&Type::Number));
                let te = self.coerce(te, &Type::Number);
                TExpr::new(TExprKind::Coerce(Coercion::NumToI32, Box::new(te)), Type::Int, span)
            }
            StdFn::ClipboardLastError => {
                if !args.is_empty() {
                    self.err(code::ARG_COUNT, span, "`lastError` takes no arguments");
                }
                TExpr::new(TExprKind::Rt("clipboard_last_error", Vec::new()), Type::String.nullable(), span)
            }
            StdFn::DialogAlert => {
                if args.is_empty() || args.len() > 2 {
                    self.err(code::ARG_COUNT, span, "`alert` takes a message and an optional done callback");
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let message = self.expr_with(&args[0], &Type::String);
                let message = self.coerce(message, &Type::String);
                match args.get(1) {
                    Some(done) => {
                        let cb = self.callback(done, &[], Some(Type::Void)).0;
                        TExpr::new(TExprKind::DialogCall("dialog_alert", Box::new(message), Box::new(cb)), Type::Void, span)
                    }
                    // Without `done`, `alert` returns a `Promise<void>`
                    // (SPEC.md §4.5) that settles when the user closes it.
                    None => self.host_promise(Type::Void, span, |_, cb| {
                        TExpr::new(TExprKind::DialogCall("dialog_alert", Box::new(message), Box::new(cb)), Type::Void, span)
                    }),
                }
            }
            StdFn::DialogConfirm => {
                if args.is_empty() || args.len() > 2 {
                    self.err(code::ARG_COUNT, span, "`confirm` takes a message and an optional done callback");
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let message = self.expr_with(&args[0], &Type::String);
                let message = self.coerce(message, &Type::String);
                match args.get(1) {
                    Some(done) => {
                        let (cb, _) = self.callback(done, &[Type::Bool], Some(Type::Void));
                        TExpr::new(TExprKind::DialogCall("dialog_confirm", Box::new(message), Box::new(cb)), Type::Void, span)
                    }
                    None => self.host_promise(Type::Bool, span, |_, cb| {
                        TExpr::new(TExprKind::DialogCall("dialog_confirm", Box::new(message), Box::new(cb)), Type::Void, span)
                    }),
                }
            }
            StdFn::DialogPrompt => {
                if args.is_empty() || args.len() > 2 {
                    self.err(code::ARG_COUNT, span, "`prompt` takes a message and an optional done callback");
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let message = self.expr_with(&args[0], &Type::String);
                let message = self.coerce(message, &Type::String);
                match args.get(1) {
                    Some(done) => {
                        let (cb, _) = self.callback(done, &[Type::String.nullable()], Some(Type::Void));
                        TExpr::new(TExprKind::DialogCall("dialog_prompt", Box::new(message), Box::new(cb)), Type::Void, span)
                    }
                    None => self.host_promise(Type::String.nullable(), span, |_, cb| {
                        TExpr::new(TExprKind::DialogCall("dialog_prompt", Box::new(message), Box::new(cb)), Type::Void, span)
                    }),
                }
            }
            StdFn::NetFetch => {
                self.require_net_capability(span);
                if args.len() != 2 && args.len() != 3 {
                    self.err(code::ARG_COUNT, span, "`fetch` takes a url, options, and an optional done callback");
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let url = self.expr_with(&args[0], &Type::String);
                let url = self.coerce(url, &Type::String);
                let (method, headers, body) = self.net_fetch_options(&args[1]);

                let response_sid = self.response_struct();
                let Some(done) = args.get(2) else {
                    // Without `done`, `fetch` returns a `Promise<Response>`.
                    return self.host_promise(Type::Struct(response_sid), span, |c, cb| {
                        c.net_fetch_call(url, method, headers, body, cb, response_sid, span)
                    });
                };
                let (cb, _) = self.callback(done, &[Type::Struct(response_sid)], Some(Type::Void));
                self.net_fetch_call(url, method, headers, body, cb, response_sid, span)
            }
            StdFn::HubListApps => {
                self.require_capability(CAP_HUB_MANAGE, span);
                if !args.is_empty() {
                    self.err(code::ARG_COUNT, span, "`listApps` takes no arguments");
                }
                TExpr::new(TExprKind::Rt("hub_list_apps", Vec::new()), Type::String.nullable(), span)
            }
            StdFn::HubLaunch => {
                self.require_capability(CAP_HUB_MANAGE, span);
                if !one_arg(self, "launch") {
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let id = self.expr_with(&args[0], &Type::String);
                let id = self.coerce(id, &Type::String);
                TExpr::new(TExprKind::Rt("hub_launch", vec![id]), Type::Void, span)
            }
            StdFn::HubSetGrant => {
                self.require_capability(CAP_HUB_MANAGE, span);
                if args.len() != 3 {
                    self.err(code::ARG_COUNT, span, "`setGrant` takes an app id, a capability name and `allowed`");
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let id = self.expr_with(&args[0], &Type::String);
                let id = self.coerce(id, &Type::String);
                let capability = self.expr_with(&args[1], &Type::String);
                let capability = self.coerce(capability, &Type::String);
                let allowed = self.expr_with(&args[2], &Type::Bool);
                let allowed = self.coerce(allowed, &Type::Bool);
                TExpr::new(TExprKind::Rt("hub_set_grant", vec![id, capability, allowed]), Type::Void, span)
            }
            StdFn::HubBlock => {
                self.require_capability(CAP_HUB_MANAGE, span);
                if !one_arg(self, "block") {
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let id = self.expr_with(&args[0], &Type::String);
                let id = self.coerce(id, &Type::String);
                TExpr::new(TExprKind::Rt("hub_block", vec![id]), Type::Void, span)
            }
            StdFn::HubUnblock => {
                self.require_capability(CAP_HUB_MANAGE, span);
                if !one_arg(self, "unblock") {
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let id = self.expr_with(&args[0], &Type::String);
                let id = self.coerce(id, &Type::String);
                TExpr::new(TExprKind::Rt("hub_unblock", vec![id]), Type::Void, span)
            }
            StdFn::HubLastError => {
                if !args.is_empty() {
                    self.err(code::ARG_COUNT, span, "`lastError` takes no arguments");
                }
                TExpr::new(TExprKind::Rt("hub_last_error", Vec::new()), Type::String.nullable(), span)
            }
            StdFn::HubListGroups => {
                self.require_capability(CAP_HUB_MANAGE, span);
                if !args.is_empty() {
                    self.err(code::ARG_COUNT, span, "`listGroups` takes no arguments");
                }
                TExpr::new(TExprKind::Rt("hub_list_groups", Vec::new()), Type::String.nullable(), span)
            }
            StdFn::HubCreateGroup => {
                self.require_capability(CAP_HUB_MANAGE, span);
                if !one_arg(self, "createGroup") {
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let name = self.expr_with(&args[0], &Type::String);
                let name = self.coerce(name, &Type::String);
                TExpr::new(TExprKind::Rt("hub_create_group", vec![name]), Type::Void, span)
            }
            StdFn::HubSetGroup => {
                self.require_capability(CAP_HUB_MANAGE, span);
                if args.len() != 3 {
                    self.err(code::ARG_COUNT, span, "`setGroup` takes an app id, a group name and `member`");
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let id = self.expr_with(&args[0], &Type::String);
                let id = self.coerce(id, &Type::String);
                let group = self.expr_with(&args[1], &Type::String);
                let group = self.coerce(group, &Type::String);
                let member = self.expr_with(&args[2], &Type::Bool);
                let member = self.coerce(member, &Type::Bool);
                TExpr::new(TExprKind::Rt("hub_set_group", vec![id, group, member]), Type::Void, span)
            }
            StdFn::HubRemove => {
                self.require_capability(CAP_HUB_MANAGE, span);
                if !one_arg(self, "remove") {
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let id = self.expr_with(&args[0], &Type::String);
                let id = self.coerce(id, &Type::String);
                TExpr::new(TExprKind::Rt("hub_remove", vec![id]), Type::Void, span)
            }
            StdFn::HubAppInfo => {
                self.require_capability(CAP_HUB_MANAGE, span);
                if !one_arg(self, "appInfo") {
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let id = self.expr_with(&args[0], &Type::String);
                let id = self.coerce(id, &Type::String);
                TExpr::new(TExprKind::Rt("hub_app_info", vec![id]), Type::String.nullable(), span)
            }
            StdFn::HubPin => {
                self.require_capability(CAP_HUB_MANAGE, span);
                if args.len() != 2 {
                    self.err(code::ARG_COUNT, span, "`pin` takes an app id and a version (`\"\"` removes the pin)");
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let id = self.expr_with(&args[0], &Type::String);
                let id = self.coerce(id, &Type::String);
                let version = self.expr_with(&args[1], &Type::String);
                let version = self.coerce(version, &Type::String);
                TExpr::new(TExprKind::Rt("hub_pin", vec![id, version]), Type::Void, span)
            }
            StdFn::HubBlockPublisher | StdFn::HubUnblockPublisher => {
                self.require_capability(CAP_HUB_MANAGE, span);
                let (name, rt_fn) = match f {
                    StdFn::HubBlockPublisher => ("blockPublisher", "hub_block_publisher"),
                    _ => ("unblockPublisher", "hub_unblock_publisher"),
                };
                if !one_arg(self, name) {
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let key = self.expr_with(&args[0], &Type::String);
                let key = self.coerce(key, &Type::String);
                TExpr::new(TExprKind::Rt(rt_fn, vec![key]), Type::Void, span)
            }
            StdFn::FilesRead | StdFn::FilesWrite | StdFn::FilesList | StdFn::FilesStat | StdFn::FilesRemove => self.files_call(f, args, span),
            StdFn::HubSearch | StdFn::HubInstall | StdFn::HubCheckUpdates | StdFn::HubUpdate => {
                self.require_capability(CAP_HUB_MANAGE, span);
                let (name, rt_fn, what) = match f {
                    StdFn::HubSearch => ("search", "hub_search", "a query"),
                    StdFn::HubCheckUpdates => ("checkUpdates", "hub_check_updates", "an app id (`\"\"` for every app)"),
                    StdFn::HubUpdate => ("update", "hub_update", "an app id"),
                    _ => ("install", "hub_install", "an app id"),
                };
                if args.is_empty() || args.len() > 2 {
                    self.err(code::ARG_COUNT, span, format!("`{name}` takes {what} and an optional done callback"));
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let text = self.expr_with(&args[0], &Type::String);
                let text = self.coerce(text, &Type::String);
                let Some(done) = args.get(1) else {
                    // Without `done`, the call returns a `Promise<string | null>`.
                    return self.host_promise(Type::String.nullable(), span, |_, cb| {
                        TExpr::new(TExprKind::DialogCall(rt_fn, Box::new(text), Box::new(cb)), Type::Void, span)
                    });
                };
                let (cb, _) = self.callback(done, &[Type::String.nullable()], Some(Type::Void));
                // The same request/completion shape as `dialog.prompt`
                // (SPEC.md §8.4): one string argument, then a callback
                // that gets `string | null`.
                TExpr::new(TExprKind::DialogCall(rt_fn, Box::new(text), Box::new(cb)), Type::Void, span)
            }
        }
    }

    /// The `net.fetch` call with a `done` callback `cb` (SPEC.md §8.4):
    /// the wrapper the runtime invokes when the completion arrives decodes
    /// the 4 fields the host stashed (`net_result_*`), builds the
    /// `Response`, and calls `cb`.
    #[allow(clippy::too_many_arguments)]
    fn net_fetch_call(&mut self, url: TExpr, method: TExpr, headers: TExpr, body: TExpr, cb: TExpr, response_sid: crate::types::StructId, span: Span) -> TExpr {
        let rt = |name: &'static str, ty: Type| TExpr::new(TExprKind::Rt(name, Vec::new()), ty, span);
        let resp = TExpr::new(
            TExprKind::StructLit(
                response_sid,
                vec![
                    rt("net_result_ok", Type::Bool),
                    rt("net_result_status", Type::Number),
                    rt("net_result_text", Type::String),
                    rt("net_result_error", Type::String.nullable()),
                ],
            ),
            Type::Struct(response_sid),
            span,
        );
        let call = TExpr::new(TExprKind::CallClosure(Box::new(cb), vec![resp]), Type::Void, span);
        let wrapper = self.synthetic_closure("<net_fetch_done>", vec![TStmt::Return(Some(call))], Type::Void, span);
        TExpr::new(
            TExprKind::NetFetchCall(Box::new(url), Box::new(method), Box::new(headers), Box::new(body), Box::new(wrapper)),
            Type::Void,
            span,
        )
    }

    /// A `plinth:files` call (core 1.11, `docs/STORAGE.md` §2, §3). With a
    /// `done` callback, `done(error, value)` (`done(error)` for `write` and
    /// `remove`); without it, a `Promise` of the value that rejects with an
    /// `Error` whose message is the error text (for example
    /// `"denied:refused"` or `"not-found"`). Both forms use one wrapper
    /// closure that reads the completion list with `net_result_*`.
    fn files_call(&mut self, f: StdFn, args: &[Expr], span: Span) -> TExpr {
        self.require_capability(CAP_FILES_PRIVATE, span);
        let entry = Type::Struct(self.file_entry_struct());
        // (name, op code of `files_call`, the value type, takes text)
        let (name, op, value_ty, has_text) = match f {
            StdFn::FilesRead => ("read", 0, Type::String, false),
            StdFn::FilesWrite => ("write", 1, Type::Void, true),
            StdFn::FilesList => ("list", 2, Type::Array(Box::new(entry.clone())), false),
            StdFn::FilesStat => ("stat", 3, entry.nullable(), false),
            _ => ("remove", 4, Type::Void, false),
        };
        let fixed = if has_text { 2 } else { 1 };
        if args.len() != fixed && args.len() != fixed + 1 {
            let what = if has_text { "a path, the text" } else { "a path" };
            self.err(code::ARG_COUNT, span, format!("`{name}` takes {what} and an optional done callback"));
            return TExpr::new(TExprKind::Null, Type::Error, span);
        }
        let path = self.expr_with(&args[0], &Type::String);
        let path = self.coerce(path, &Type::String);
        let text = if has_text {
            let t = self.expr_with(&args[1], &Type::String);
            let t = self.coerce(t, &Type::String);
            self.coerce(t, &Type::String.nullable())
        } else {
            TExpr::new(TExprKind::Null, Type::String.nullable(), span)
        };
        let done = args.get(fixed).map(|d| {
            let mut params = vec![Type::String.nullable()];
            if value_ty != Type::Void {
                params.push(value_ty.clone());
            }
            self.callback(d, &params, Some(Type::Void))
        });

        // The wrapper: `e = error; t = text; v = decode(t); ...`.
        let fid = self.prog.new_func(FuncDef {
            name: format!("<files_{name}_done>"),
            kind: FuncKind::Closure,
            params: Vec::new(),
            ret: Type::Void,
            body: Vec::new(),
            span,
        });
        let e_ty = Type::String.nullable();
        let ev = self.temp(e_ty.clone());
        let tv = self.temp(Type::String);
        let e = TExpr::new(TExprKind::Var(ev), e_ty.clone(), span);
        let t = TExpr::new(TExprKind::Var(tv), Type::String, span);
        let mut body = vec![
            TStmt::Let(ev, Some(TExpr::new(TExprKind::Rt("net_result_error", Vec::new()), e_ty.clone(), span))),
            TStmt::Let(tv, Some(TExpr::new(TExprKind::Rt("net_result_text", Vec::new()), Type::String, span))),
        ];
        let value = match f {
            StdFn::FilesRead => Some(t),
            StdFn::FilesStat => Some(self.json_parse_value(t, &value_ty, span)),
            StdFn::FilesList => {
                // `JSON.parse<FileEntry[]>(t) ?? []`.
                let parsed = self.json_parse_value(t, &value_ty, span);
                let lv = self.temp(parsed.ty.clone());
                let l = TExpr::new(TExprKind::Var(lv), parsed.ty.clone(), span);
                body.push(TStmt::Let(lv, Some(parsed)));
                let is_null = TExpr::new(TExprKind::IsNull(Box::new(l.clone())), Type::Bool, span);
                let empty = TExpr::new(TExprKind::ArrayLit(Vec::new()), value_ty.clone(), span);
                let some = TExpr::new(TExprKind::Coerce(Coercion::Retag, Box::new(l)), value_ty.clone(), span);
                Some(TExpr::new(TExprKind::Cond(Box::new(is_null), Box::new(empty), Box::new(some)), value_ty.clone(), span))
            }
            _ => None,
        };
        let vv = value.map(|v| {
            let vv = self.temp(value_ty.clone());
            body.push(TStmt::Let(vv, Some(v)));
            TExpr::new(TExprKind::Var(vv), value_ty.clone(), span)
        });
        let result = match done {
            Some((cb, n)) => {
                let mut call_args = vec![e];
                call_args.extend(vv);
                call_args.truncate(n);
                body.push(TStmt::Expr(TExpr::new(TExprKind::CallClosure(Box::new(cb), call_args), Type::Void, span)));
                None
            }
            None => {
                let info = self.promise_info(&value_ty);
                let pty = Type::Struct(info.sid);
                let pv = self.temp(pty.clone());
                let p = TExpr::new(TExprKind::Var(pv), pty.clone(), span);
                let (_, reject) = self.promise_then_reject();
                let resolved = vv.unwrap_or_else(|| TExpr::new(TExprKind::Bool(false), Type::Bool, span));
                let resolve = TStmt::Expr(TExpr::new(TExprKind::Call(info.resolve, vec![p.clone(), resolved]), Type::Void, span));
                let err = self.new_error(e.clone());
                let base = self.as_base(p.clone());
                let reject = TStmt::Expr(TExpr::new(TExprKind::Call(reject, vec![base, err]), Type::Void, span));
                let is_null = TExpr::new(TExprKind::IsNull(Box::new(e)), Type::Bool, span);
                body.push(TStmt::If(is_null, vec![resolve], vec![reject]));
                Some((pv, p, info))
            }
        };
        super::asyncfn::reown(&mut self.prog, &body, fid, None);
        self.prog.funcs[fid as usize].body = body;
        let ft = crate::types::FuncType { params: Vec::new(), required: 0, ret: Type::Void };
        let wrapper = TExpr::new(TExprKind::Closure(fid), Type::Func(std::rc::Rc::new(ft)), span);
        let call = TExpr::new(TExprKind::FilesCall(op, Box::new(path), Box::new(text), Box::new(wrapper)), Type::Void, span);
        match result {
            None => call,
            Some((pv, p, info)) => {
                let pty = Type::Struct(info.sid);
                TExpr::new(TExprKind::Block(vec![TStmt::Let(pv, Some(self.new_promise(&info, span))), TStmt::Expr(call)], Box::new(p)), pty, span)
            }
        }
    }

    /// `fetch`'s `options`: `null`, or an object literal with optional
    /// `method`, `headers` (a `Map<string, string>`) and `body` fields
    /// (SPEC.md §8.5). A general expression is not supported yet (only a
    /// literal, so the checker never needs a nullable-struct field read);
    /// returns `(method, headers, body)` typed expressions with the
    /// defaults already filled in.
    fn net_fetch_options(&mut self, e: &Expr) -> (TExpr, TExpr, TExpr) {
        let span = e.span;
        let default_method = || TExpr::new(TExprKind::Str(DEFAULT_METHOD.to_owned()), Type::String, span);
        let default_body = || TExpr::new(TExprKind::Null, Type::String.nullable(), span);
        let empty_headers = |c: &mut Self| {
            let (k, v) = (Type::String, Type::String);
            let sid = c.map_struct(&k, &v);
            let arr = |elem: &Type| TExpr::new(TExprKind::ArrayLit(Vec::new()), Type::Array(Box::new(elem.clone())), span);
            TExpr::new(TExprKind::StructLit(sid, vec![arr(&k), arr(&v)]), Type::Map(Box::new(k), Box::new(v)), span)
        };
        match &e.kind {
            ExprKind::Null => (default_method(), empty_headers(self), default_body()),
            ExprKind::Object(props) => {
                let (mut method, mut headers, mut body) = (None, None, None);
                for p in props {
                    let ObjProp::Field(name, value, fspan) = p else {
                        self.err(code::UNSUPPORTED, e.span, "`fetch`'s options cannot use a spread");
                        continue;
                    };
                    match name.as_str() {
                        "method" => {
                            let te = self.expr_with(value, &Type::String);
                            method = Some(self.coerce(te, &Type::String));
                        }
                        "headers" => {
                            let want = Type::Map(Box::new(Type::String), Box::new(Type::String));
                            let te = self.expr_with(value, &want);
                            headers = Some(self.coerce(te, &want));
                        }
                        "body" => {
                            let te = self.expr_with(value, &Type::String.nullable());
                            body = Some(self.coerce(te, &Type::String.nullable()));
                        }
                        _ => self.err_help(
                            code::NO_PROPERTY,
                            *fspan,
                            format!("fetch options has no `{name}` property"),
                            "use `method`, `headers` or `body`",
                        ),
                    }
                }
                (method.unwrap_or_else(default_method), headers.unwrap_or_else(|| empty_headers(self)), body.unwrap_or_else(default_body))
            }
            _ => {
                self.err_help(
                    code::UNSUPPORTED,
                    e.span,
                    "`fetch`'s `options` must be `null` or an object literal",
                    "write `null` or `{ method: \"POST\", body: \"...\" }`",
                );
                (default_method(), empty_headers(self), default_body())
            }
        }
    }

    pub(super) fn std_obj_call(
        &mut self,
        o: StdObj,
        prop: &str,
        prop_span: Span,
        type_args: &[ast::TypeAnn],
        args: &[Expr],
        span: Span,
    ) -> TExpr {
        match o {
            StdObj::Navigate => match prop {
                "push" => {
                    if args.len() != 1 {
                        self.err(code::ARG_COUNT, span, "`navigate.push` takes one argument: a screen name literal");
                        return TExpr::new(TExprKind::Null, Type::Error, span);
                    }
                    match &args[0].kind {
                        ExprKind::Str(name) => {
                            self.navigations.push((name.clone(), args[0].span, false));
                            TExpr::new(TExprKind::NavigatePush(name.clone()), Type::Void, span)
                        }
                        _ => {
                            self.err_help(
                                code::BAD_NAVIGATE,
                                args[0].span,
                                "`navigate.push` takes a screen name literal",
                                "write `navigate.push(\"detail\")`",
                            );
                            TExpr::new(TExprKind::Null, Type::Error, span)
                        }
                    }
                }
                "back" => {
                    if !args.is_empty() {
                        self.err(code::ARG_COUNT, span, "`navigate.back` takes no arguments");
                    }
                    TExpr::new(TExprKind::NavigateBack, Type::Void, span)
                }
                _ => {
                    self.err(code::NO_PROPERTY, prop_span, format!("`navigate.{prop}` does not exist; use `.push` or `.back`"));
                    TExpr::new(TExprKind::Null, Type::Error, span)
                }
            },
            StdObj::Console => {
                if prop != "log" {
                    self.err(code::NO_PROPERTY, prop_span, format!("`console.{prop}` does not exist; use `console.log`"));
                }
                let mut msg: Option<TExpr> = None;
                for (i, a) in args.iter().enumerate() {
                    let te = self.expr(a, None);
                    let s = self.to_str(te);
                    msg = Some(match msg {
                        None => s,
                        Some(prev) => {
                            let sep = TExpr::new(TExprKind::Str(" ".into()), Type::String, span);
                            let with_sep = TExpr::new(TExprKind::Concat(Box::new(prev), Box::new(sep)), Type::String, span);
                            TExpr::new(TExprKind::Concat(Box::new(with_sep), Box::new(s)), Type::String, span)
                        }
                    });
                    let _ = i;
                }
                let msg = msg.unwrap_or_else(|| TExpr::new(TExprKind::Str(String::new()), Type::String, span));
                TExpr::new(TExprKind::Rt("log", vec![msg]), Type::Void, span)
            }
            StdObj::Math => {
                let nums: Vec<TExpr> = args
                    .iter()
                    .map(|a| {
                        let te = self.expr(a, Some(&Type::Number));
                        self.coerce(te, &Type::Number)
                    })
                    .collect();
                let (arity, kind): (usize, Result<MathOp, &'static str>) = match prop {
                    "floor" => (1, Ok(MathOp::Floor)),
                    "ceil" => (1, Ok(MathOp::Ceil)),
                    "trunc" => (1, Ok(MathOp::Trunc)),
                    "abs" => (1, Ok(MathOp::Abs)),
                    "sqrt" => (1, Ok(MathOp::Sqrt)),
                    "min" => (2, Ok(MathOp::Min)),
                    "max" => (2, Ok(MathOp::Max)),
                    "round" => (1, Err("f64_round")),
                    "pow" => (2, Err("f64_pow")),
                    _ => {
                        self.err(code::NO_PROPERTY, prop_span, format!("`Math.{prop}` is not available in Plinth TS"));
                        return TExpr::new(TExprKind::Num(0.0), Type::Error, span);
                    }
                };
                if nums.len() != arity {
                    self.err(code::ARG_COUNT, span, format!("`Math.{prop}` takes {arity} argument(s)"));
                    return TExpr::new(TExprKind::Num(0.0), Type::Error, span);
                }
                match kind {
                    Ok(op) => TExpr::new(TExprKind::MathOp(op, nums), Type::Number, span),
                    Err(rt) => TExpr::new(TExprKind::Rt(rt, nums), Type::Number, span),
                }
            }
            StdObj::Kv => {
                self.require_capability(CAP_STORE_KV, span);
                match prop {
                    "get" => {
                        if args.len() != 1 {
                            self.err(code::ARG_COUNT, span, "`kv.get` takes one argument");
                            return TExpr::new(TExprKind::Null, Type::Error, span);
                        }
                        let key = self.expr_with(&args[0], &Type::String);
                        let key = self.coerce(key, &Type::String);
                        TExpr::new(TExprKind::Rt("kv_get", vec![key]), Type::String.nullable(), span)
                    }
                    "set" => {
                        if args.len() != 2 {
                            self.err(code::ARG_COUNT, span, "`kv.set` takes a key and a value");
                            return TExpr::new(TExprKind::Null, Type::Error, span);
                        }
                        let key = self.expr_with(&args[0], &Type::String);
                        let key = self.coerce(key, &Type::String);
                        let value = self.expr_with(&args[1], &Type::String);
                        let value = self.coerce(value, &Type::String);
                        TExpr::new(TExprKind::Rt("kv_set", vec![key, value]), Type::Void, span)
                    }
                    "remove" => {
                        if args.len() != 1 {
                            self.err(code::ARG_COUNT, span, "`kv.remove` takes one argument");
                            return TExpr::new(TExprKind::Null, Type::Error, span);
                        }
                        let key = self.expr_with(&args[0], &Type::String);
                        let key = self.coerce(key, &Type::String);
                        TExpr::new(TExprKind::Rt("kv_delete", vec![key]), Type::Void, span)
                    }
                    "keys" => {
                        if !args.is_empty() {
                            self.err(code::ARG_COUNT, span, "`kv.keys` takes no arguments");
                        }
                        TExpr::new(TExprKind::Rt("kv_keys", Vec::new()), Type::Array(Box::new(Type::String)), span)
                    }
                    "lastError" => {
                        if !args.is_empty() {
                            self.err(code::ARG_COUNT, span, "`kv.lastError` takes no arguments");
                        }
                        TExpr::new(TExprKind::Rt("kv_last_error", Vec::new()), Type::String.nullable(), span)
                    }
                    _ => {
                        self.err(code::NO_PROPERTY, prop_span, format!("`kv.{prop}` does not exist"));
                        TExpr::new(TExprKind::Null, Type::Error, span)
                    }
                }
            }
            StdObj::Json => match prop {
                "stringify" => {
                    if args.len() != 1 {
                        self.err_help(
                            code::ARG_COUNT,
                            span,
                            "`JSON.stringify` takes one argument",
                            "the `replacer` and `space` arguments are not supported yet",
                        );
                        return TExpr::new(TExprKind::Str(String::new()), Type::Error, span);
                    }
                    let value = self.expr(&args[0], None);
                    self.json_stringify_value(value, span)
                }
                "parse" => {
                    if type_args.len() != 1 {
                        self.err_help(
                            code::GENERIC_USER,
                            span,
                            "`JSON.parse` needs an explicit type argument",
                            "write `JSON.parse<YourType>(text)`",
                        );
                        return TExpr::new(TExprKind::Null, Type::Error, span);
                    }
                    if args.len() != 1 {
                        self.err(code::ARG_COUNT, span, "`JSON.parse` takes one argument");
                        return TExpr::new(TExprKind::Null, Type::Error, span);
                    }
                    let ty = self.resolve_type(&type_args[0]);
                    let text = self.expr_with(&args[0], &Type::String);
                    let text = self.coerce(text, &Type::String);
                    self.json_parse_value(text, &ty, span)
                }
                _ => {
                    self.err_help(code::NO_PROPERTY, prop_span, format!("`JSON.{prop}` does not exist"), "use `JSON.stringify`/`JSON.parse`");
                    TExpr::new(TExprKind::Null, Type::Error, span)
                }
            },
        }
    }

    /// `export default app({ accent, screens, primary })` in app/main.tsx.
    pub(super) fn app_config(&mut self, e: &Expr) {
        let ExprKind::Call { callee, args, .. } = &e.kind else {
            self.err_help(code::BAD_APP, e.span, "app/main.tsx must export `app({...})`", "write `export default app({...})`");
            return;
        };
        let is_app = matches!(&callee.kind, ExprKind::Ident(n) if matches!(self.lookup(n), Some(Binding::Std(StdFn::App))));
        if !is_app {
            self.err_help(code::BAD_APP, e.span, "app/main.tsx must export `app({...})`", "import `app` from \"plinth:ui\"");
            return;
        }
        self.app_seen = true;
        let Some(Expr { kind: ExprKind::Object(props), span: obj_span }) = args.first() else {
            self.err(code::BAD_APP, e.span, "`app` takes an object literal");
            return;
        };
        let mut screens: Vec<(String, Option<String>, FuncId, Span)> = Vec::new();
        let mut primary: Option<Vec<(String, Span)>> = None;
        for p in props {
            let ObjProp::Field(name, value, fspan) = p else {
                self.err(code::BAD_APP, *obj_span, "spread is not allowed in the app config");
                continue;
            };
            match name.as_str() {
                "accent" => match &value.kind {
                    ExprKind::Str(a) if controls::ACCENTS.contains(&a.as_str()) => self.prog.accent = Some(a.clone()),
                    _ => self.err_help(
                        code::BAD_APP,
                        value.span,
                        "`accent` must be one of the named accent colors",
                        format!("use one of: {}", controls::ACCENTS.join(", ")),
                    ),
                },
                "screens" => {
                    let ExprKind::Object(defs) = &value.kind else {
                        self.err(code::BAD_APP, value.span, "`screens` must be an object literal");
                        continue;
                    };
                    for d in defs {
                        let ObjProp::Field(sname, def, sspan) = d else { continue };
                        if let Some(s) = self.screen_def(sname, def) {
                            screens.push((sname.clone(), s.0, s.1, *sspan));
                        }
                    }
                }
                "primary" => {
                    let ExprKind::Array(items) = &value.kind else {
                        self.err(code::BAD_APP, value.span, "`primary` must be an array of screen names");
                        continue;
                    };
                    let mut names = Vec::new();
                    for (_, it) in items {
                        match &it.kind {
                            ExprKind::Str(s) => names.push((s.clone(), it.span)),
                            _ => self.err(code::BAD_APP, it.span, "a primary screen must be a name literal"),
                        }
                    }
                    primary = Some(names);
                }
                other => self.err(code::BAD_APP, *fspan, format!("unknown app option `{other}`")),
            }
        }
        if screens.is_empty() {
            self.err(code::BAD_APP, *obj_span, "the app needs at least one screen");
            return;
        }
        // Primary screens first (their index doubles as their tab position),
        // then the non-primary screens, pushable with `navigate.push`
        // (SPEC.md §6.2, UI API 1.2).
        let primary_names: Vec<String> = match primary {
            Some(names) => {
                for (n, sp) in &names {
                    if !screens.iter().any(|s| s.0 == *n) {
                        self.err(code::BAD_APP, *sp, format!("there is no screen named \"{n}\""));
                    }
                }
                names.into_iter().map(|(n, _)| n).collect()
            }
            None => screens.iter().map(|s| s.0.clone()).collect(),
        };
        let mut order: Vec<(String, bool)> = primary_names.iter().map(|n| (n.clone(), true)).collect();
        for s in &screens {
            if !primary_names.contains(&s.0) {
                order.push((s.0.clone(), false));
            }
        }
        for (name, is_primary) in order {
            if let Some((n, icon, component, _)) = screens.iter().find(|s| s.0 == name) {
                self.prog.screens.push(Screen { name: n.clone(), icon: icon.clone(), component: *component, primary: is_primary });
            }
        }
    }

    fn screen_def(&mut self, name: &str, def: &Expr) -> Option<(Option<String>, FuncId)> {
        let ExprKind::Object(fields) = &def.kind else {
            self.err(code::BAD_APP, def.span, format!("the screen \"{name}\" must be an object literal"));
            return None;
        };
        let mut icon = None;
        let mut component = None;
        for f in fields {
            let ObjProp::Field(key, v, fspan) = f else { continue };
            match key.as_str() {
                "title" => {
                    if !matches!(v.kind, ExprKind::Str(_)) {
                        self.err(code::BAD_APP, v.span, "`title` must be a string literal");
                    }
                }
                "icon" => match &v.kind {
                    ExprKind::Str(i) if controls::ICONS.contains(&i.as_str()) => icon = Some(i.clone()),
                    _ => self.err_help(
                        code::BAD_APP,
                        v.span,
                        "`icon` must be a name from the runtime icon set",
                        format!("use one of: {}", controls::ICONS.join(", ")),
                    ),
                },
                "component" => {
                    let ExprKind::Ident(fname) = &v.kind else {
                        self.err(code::BAD_APP, v.span, "`component` must be the name of a function component");
                        continue;
                    };
                    match self.lookup(fname) {
                        Some(Binding::Func(fid)) => {
                            let ft = self.func_type(fid, v.span);
                            if !ft.params.is_empty() || (ft.ret != Type::Element && !ft.ret.is_error()) {
                                let msg = format!("a screen component takes no props and returns an element; `{fname}` has type `{}`", self.show(&Type::Func(ft.clone())));
                                self.err(code::BAD_APP, v.span, msg);
                            }
                            component = Some(fid);
                        }
                        _ => self.err(code::BAD_APP, v.span, format!("`{fname}` is not a function component")),
                    }
                }
                other => self.err(code::BAD_APP, *fspan, format!("unknown screen option `{other}`")),
            }
        }
        match component {
            Some(c) => Some((icon, c)),
            None => {
                self.err(code::BAD_APP, def.span, format!("the screen \"{name}\" needs a `component`"));
                None
            }
        }
    }
}

impl Checker<'_> {
    /// One Canvas shape (UI API 1.10): the fields joined with U+001F, the
    /// kind letter first. Numbers use the JSON number form; a color is a
    /// theme token name, checked at compile time.
    fn shape_call(&mut self, f: StdFn, args: &[Expr], span: Span) -> TExpr {
        let (name, kind, nums, has_color, extra) = match f {
            StdFn::ShapeRect => ("rect", "r", 4, true, None),
            StdFn::ShapeCircle => ("circle", "c", 3, true, None),
            StdFn::ShapeLine => ("line", "l", 4, true, Some("width")),
            _ => ("canvasText", "t", 2, true, Some("size")),
        };
        let text_arg = f == StdFn::ShapeText;
        let want = nums + usize::from(has_color) + usize::from(text_arg);
        if args.len() < want || args.len() > want + usize::from(extra.is_some()) {
            let usage = match f {
                StdFn::ShapeRect => "`rect(x, y, width, height, color)`",
                StdFn::ShapeCircle => "`circle(cx, cy, r, color)`",
                StdFn::ShapeLine => "`line(x1, y1, x2, y2, color, width?)`",
                _ => "`canvasText(x, y, text, color, size?)`",
            };
            self.err(code::ARG_COUNT, span, format!("`{name}` takes {usage}"));
            return TExpr::new(TExprKind::Str(String::new()), Type::String, span);
        }
        let num = |c: &mut Self, e: &Expr| {
            let te = c.expr_with(e, &Type::Number);
            let te = c.coerce(te, &Type::Number);
            TExpr::new(TExprKind::Rt("json_num_str", vec![te]), Type::String, span)
        };
        let colors = Type::str_lits(crate::controls::COLOR_NAMES.iter().map(|s| s.to_string()).collect());
        let mut parts = vec![TExpr::new(TExprKind::Str(kind.into()), Type::String, span)];
        let mut i = 0;
        for _ in 0..nums {
            parts.push(num(self, &args[i]));
            i += 1;
        }
        let text = if text_arg {
            let te = self.expr_with(&args[i], &Type::String);
            i += 1;
            Some(self.coerce(te, &Type::String))
        } else {
            None
        };
        let color = self.expr_with(&args[i], &colors);
        parts.push(self.coerce(color, &Type::String));
        i += 1;
        if extra.is_some() {
            parts.push(match args.get(i) {
                Some(e) => num(self, e),
                None => TExpr::new(TExprKind::Str(if f == StdFn::ShapeLine { "1".into() } else { "12".into() }), Type::String, span),
            });
        }
        if let Some(t) = text {
            parts.push(t);
        }
        self.join_parts(parts, "\u{1f}", span)
    }
}
