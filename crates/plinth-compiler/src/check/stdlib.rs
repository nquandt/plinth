//! The `plinth:*` modules (SPEC.md §4.7): names, calls and the `app()`
//! entry. `std/*.d.ts` declares the same names for the editor; the test
//! `std_typings_match` keeps them in sync.

use super::{Binding, Checker, StdFn, StdModule, StdObj};
use crate::ast::{self, Expr, ExprKind, ObjProp};
use crate::controls;
use crate::diag::{Span, code};
use crate::tir::*;
use crate::types::Type;

/// The value and type names that each std module exports.
pub const UI_NAMES: &[&str] = &[
    "signal", "computed", "effect", "app", "navigate", "Signal", "Computed", "Accent", "IconName", "ScreenDef", "AppConfig",
    "App", "Tone", "Align", "Group", "Screen", "Section", "Text", "Heading", "Button", "TextField", "Toggle", "List", "Row", "Empty",
    "Checkbox", "TextArea", "Slider", "NumberField", "Picker", "Progress", "Badge",
    // UI API 1.2
    "Tabs", "Sheet", "Dialog", "Menu", "Grid", "Action",
    // UI API 1.3
    "Image", "Aspect",
];
pub const CORE_NAMES: &[&str] = &["Math", "parseNumber", "toString", "console", "int", "int", "JSON"];
pub const TIME_NAMES: &[&str] = &["now", "monotonicNow", "setTimeout", "setInterval", "clearTimeout", "clearInterval"];
pub const STORE_NAMES: &[&str] = &["kv"];
pub const CLIPBOARD_NAMES: &[&str] = &["writeText", "readText", "lastError"];
pub const DIALOG_NAMES: &[&str] = &["alert", "confirm", "prompt"];

/// The `store.kv` capability (SPEC.md §11), needed by every `plinth:store`
/// call.
pub const CAP_STORE_KV: &str = "store.kv";
/// The `clipboard.write`/`clipboard.read` capabilities, needed by
/// `plinth:clipboard`'s `writeText`/`readText` respectively.
pub const CAP_CLIPBOARD_WRITE: &str = "clipboard.write";
pub const CAP_CLIPBOARD_READ: &str = "clipboard.read";

pub fn lookup(m: StdModule, name: &str) -> Option<Binding> {
    match m {
        StdModule::Time => Some(match name {
            "now" => Binding::Std(StdFn::TimeNow),
            "monotonicNow" => Binding::Std(StdFn::TimeMonotonicNow),
            "setTimeout" => Binding::Std(StdFn::SetTimeout),
            "setInterval" => Binding::Std(StdFn::SetInterval),
            "clearTimeout" | "clearInterval" => Binding::Std(StdFn::ClearTimer),
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
                if args.len() != 2 {
                    self.err(code::ARG_COUNT, span, "`alert` takes a message and a done callback");
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let message = self.expr_with(&args[0], &Type::String);
                let message = self.coerce(message, &Type::String);
                let (cb, _) = self.callback(&args[1], &[], Some(Type::Void));
                TExpr::new(TExprKind::DialogCall("dialog_alert", Box::new(message), Box::new(cb)), Type::Void, span)
            }
            StdFn::DialogConfirm => {
                if args.len() != 2 {
                    self.err(code::ARG_COUNT, span, "`confirm` takes a message and a done callback");
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let message = self.expr_with(&args[0], &Type::String);
                let message = self.coerce(message, &Type::String);
                let (cb, _) = self.callback(&args[1], &[Type::Bool], Some(Type::Void));
                TExpr::new(TExprKind::DialogCall("dialog_confirm", Box::new(message), Box::new(cb)), Type::Void, span)
            }
            StdFn::DialogPrompt => {
                if args.len() != 2 {
                    self.err(code::ARG_COUNT, span, "`prompt` takes a message and a done callback");
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let message = self.expr_with(&args[0], &Type::String);
                let message = self.coerce(message, &Type::String);
                let (cb, _) = self.callback(&args[1], &[Type::String.nullable()], Some(Type::Void));
                TExpr::new(TExprKind::DialogCall("dialog_prompt", Box::new(message), Box::new(cb)), Type::Void, span)
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
