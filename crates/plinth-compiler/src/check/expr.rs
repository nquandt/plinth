//! Expressions.

use super::{Binding, Checker, ReactiveCtx, StdObj};
use crate::ast::{self, BinOp, Expr, ExprKind, LogicOp, ObjProp, UnOp};
use crate::diag::{Span, code};
use crate::tir::*;
use crate::types::{Field, FuncType, Type};
use std::rc::Rc;

fn bx(e: TExpr) -> Box<TExpr> {
    Box::new(e)
}

impl Checker<'_> {
    /// The "not reactive" lint (HANDOFF.md §6): a signal/computed read
    /// outside JSX, `computed` or `effect`, but inside a component body,
    /// runs once and never again. Reads inside event handlers are fine.
    fn check_reactive_read(&mut self, span: Span) {
        if self.fx.reactive == ReactiveCtx::Plain {
            self.diags.push(
                crate::diag::Diagnostic::warning(
                    code::SIGNAL_NOT_REACTIVE,
                    span,
                    "this signal read is not reactive: it runs once and will not update",
                )
                .help("move the read into the JSX or a `computed`"),
            );
        }
    }

    pub(crate) fn expr(&mut self, e: &Expr, expected: Option<&Type>) -> TExpr {
        let span = e.span;
        match &e.kind {
            ExprKind::Num(n) => {
                // A literal is exactly representable, so it may satisfy an
                // `int` context directly (SPEC.md §4.2); any other number
                // needs an explicit `int(x)`.
                let ty = match expected {
                    Some(Type::Int) => Type::Int,
                    Some(Type::Nullable(inner)) if **inner == Type::Int => Type::Int,
                    _ => Type::Number,
                };
                TExpr::new(TExprKind::Num(*n), ty, span)
            }
            ExprKind::Bool(b) => TExpr::new(TExprKind::Bool(*b), Type::Bool, span),
            ExprKind::Null => TExpr::new(TExprKind::Null, Type::Null, span),
            ExprKind::Str(s) => {
                // A literal gets a literal type when the context wants one.
                let ty = match expected {
                    Some(Type::StrLits(l)) if l.contains(s) => Type::str_lits(vec![s.clone()]),
                    Some(Type::Nullable(inner)) if matches!(&**inner, Type::StrLits(l) if l.contains(s)) => {
                        Type::str_lits(vec![s.clone()])
                    }
                    Some(Type::StrLits(_)) => Type::str_lits(vec![s.clone()]),
                    _ => Type::String,
                };
                TExpr::new(TExprKind::Str(s.clone()), ty, span)
            }
            ExprKind::Template(quasis, exprs) => {
                let mut acc: Option<TExpr> = None;
                for (i, q) in quasis.iter().enumerate() {
                    if !q.is_empty() {
                        acc = Some(self.concat(acc, TExpr::new(TExprKind::Str(q.clone()), Type::String, span)));
                    }
                    if let Some(x) = exprs.get(i) {
                        let te = self.expr(x, None);
                        let s = self.to_str(te);
                        acc = Some(self.concat(acc, s));
                    }
                }
                acc.unwrap_or_else(|| TExpr::new(TExprKind::Str(String::new()), Type::String, span))
            }
            ExprKind::Ident(name) => self.ident(name, span),
            ExprKind::Member { obj, prop, prop_span, optional } => self.member(obj, prop, *prop_span, *optional, span),
            ExprKind::Index { obj, index, optional } => {
                if *optional {
                    self.err(code::UNSUPPORTED, span, "`?.[` is not supported yet");
                }
                let o = self.expr(obj, None);
                match o.ty.clone() {
                    Type::Array(elem) => {
                        let i = self.expr(index, Some(&Type::Number));
                        let i = self.coerce(i, &Type::Number);
                        TExpr::new(TExprKind::Index(bx(o), bx(i)), *elem, span)
                    }
                    Type::Error => TExpr::new(TExprKind::Null, Type::Error, span),
                    other => {
                        let msg = format!("cannot index a value of type `{}`", self.show(&other));
                        self.err_help(code::COMPUTED_ACCESS, span, msg, "only arrays support `[i]`; use a field name");
                        TExpr::new(TExprKind::Null, Type::Error, span)
                    }
                }
            }
            ExprKind::Call { callee, type_args, args, optional } => {
                if *optional {
                    self.err(code::UNSUPPORTED, span, "`?.()` is not supported yet");
                }
                self.call(callee, type_args, args, span, expected)
            }
            ExprKind::Array(elems) => self.array_lit(elems, expected, span),
            ExprKind::Object(props) => self.object_lit(props, expected, span),
            ExprKind::Unary(op, x) => match op {
                UnOp::Not => {
                    let te = self.expr(x, None);
                    let t = self.truthy(te);
                    TExpr::new(TExprKind::Not(bx(t)), Type::Bool, span)
                }
                UnOp::Neg | UnOp::Plus => {
                    let te = self.expr(x, Some(&Type::Number));
                    let te = self.coerce(te, &Type::Number);
                    if *op == UnOp::Plus { te } else { TExpr::new(TExprKind::Neg(bx(te)), Type::Number, span) }
                }
                UnOp::Typeof => {
                    self.err_help(
                        code::ADVANCED_TYPE,
                        span,
                        "`typeof` is only allowed directly in `typeof x === \"...\"`",
                        "narrow a union: `if (typeof x === \"string\") { ... }`",
                    );
                    self.expr(x, None);
                    TExpr::new(TExprKind::Str("undefined".into()), Type::Error, span)
                }
            },
            ExprKind::Binary(op, l, r) => self.binary(*op, l, r, span),
            ExprKind::Logical(op, l, r) => self.logical(*op, l, r, span, expected),
            ExprKind::Cond(c, a, b) => {
                let ct = self.expr(c, Some(&Type::Bool));
                let ct = self.truthy(ct);
                let (when_true, when_false) = self.narrowing(&ct);
                self.push_scope();
                self.fx.scopes.last_mut().unwrap().narrow.extend(when_true);
                let at = self.expr(a, expected);
                self.pop_scope();
                self.push_scope();
                self.fx.scopes.last_mut().unwrap().narrow.extend(when_false);
                let bt = self.expr(b, expected);
                self.pop_scope();
                let ty = match expected {
                    Some(t) if !t.is_error() && self.conversion(&at.ty, t).is_some() && self.conversion(&bt.ty, t).is_some() => {
                        t.clone()
                    }
                    _ => self.join(&at.ty, &bt.ty, span),
                };
                let at = self.coerce(at, &ty);
                let bt = self.coerce(bt, &ty);
                TExpr::new(TExprKind::Cond(bx(ct), bx(at), bx(bt)), ty, span)
            }
            ExprKind::Assign { op, target, value } => self.assign(*op, target, value, span),
            ExprKind::Update { inc, prefix, target } => self.update(*inc, *prefix, target, span),
            ExprKind::Func(f) => {
                let ft = match expected {
                    Some(Type::Func(ft)) => Some(ft.clone()),
                    _ => None,
                };
                let c = self.closure(f, ft.as_deref(), "<closure>");
                match ft {
                    // A context of `() => void` drops the result.
                    Some(ft) if ft.ret == Type::Void => c,
                    _ => c,
                }
            }
            ExprKind::Jsx(el) => {
                let j = self.jsx(el);
                TExpr::new(TExprKind::Jsx(Box::new(j)), Type::Element, span)
            }
            ExprKind::New(name, type_args) => self.new_map_or_set(name, type_args, expected, span),
        }
    }

    /// `new Map<K, V>()` / `new Set<T>()` (HANDOFF.md item 5): an empty
    /// 2-field (or 1-field) struct of arrays. The type arguments may come
    /// from `expected` instead, like array literals do.
    fn new_map_or_set(&mut self, name: &str, type_args: &[ast::TypeAnn], expected: Option<&Type>, span: Span) -> TExpr {
        let empty_arr = |elem: &Type| TExpr::new(TExprKind::ArrayLit(Vec::new()), Type::Array(Box::new(elem.clone())), span);
        if name == "Map" {
            let (k, v) = match (type_args, expected) {
                ([k, v], _) => (self.resolve_type(k), self.resolve_type(v)),
                ([], Some(Type::Map(k, v))) => ((**k).clone(), (**v).clone()),
                _ => {
                    self.err_help(code::CANNOT_INFER, span, "cannot infer the key and value types of `new Map()`", "write `new Map<K, V>()`");
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
            };
            if !type_args.is_empty() && !self.valid_key_type(&k) {
                let msg = format!("a `Map` key must be `string`, `int`, `number`, `boolean` or an enum, not `{}`", self.show(&k));
                self.err(code::ADVANCED_TYPE, span, msg);
                return TExpr::new(TExprKind::Null, Type::Error, span);
            }
            if !type_args.is_empty() && !self.valid_map_value_type(&v) {
                let msg = format!("a `Map`/`Set` value of type `{}` is not supported yet", self.show(&v));
                self.err(code::ADVANCED_TYPE, span, msg);
                return TExpr::new(TExprKind::Null, Type::Error, span);
            }
            let sid = self.map_struct(&k, &v);
            let lit = TExpr::new(TExprKind::StructLit(sid, vec![empty_arr(&k), empty_arr(&v)]), Type::Map(Box::new(k), Box::new(v)), span);
            return lit;
        }
        let t = match (type_args, expected) {
            ([t], _) => self.resolve_type(t),
            ([], Some(Type::Set(t))) => (**t).clone(),
            _ => {
                self.err_help(code::CANNOT_INFER, span, "cannot infer the element type of `new Set()`", "write `new Set<T>()`");
                return TExpr::new(TExprKind::Null, Type::Error, span);
            }
        };
        if !type_args.is_empty() && !self.valid_key_type(&t) {
            let msg = format!("a `Set` element must be `string`, `int`, `number`, `boolean` or an enum, not `{}`", self.show(&t));
            self.err(code::ADVANCED_TYPE, span, msg);
            return TExpr::new(TExprKind::Null, Type::Error, span);
        }
        let sid = self.set_struct(&t);
        TExpr::new(TExprKind::StructLit(sid, vec![empty_arr(&t)]), Type::Set(Box::new(t)), span)
    }

    fn concat(&mut self, acc: Option<TExpr>, part: TExpr) -> TExpr {
        match acc {
            None => part,
            Some(a) => {
                let span = Span { end: part.span.end, ..a.span };
                TExpr::new(TExprKind::Concat(bx(a), bx(part)), Type::String, span)
            }
        }
    }

    /// The common type of two branches.
    fn join(&mut self, a: &Type, b: &Type, span: Span) -> Type {
        match (a, b) {
            _ if a == b => a.clone(),
            (Type::Error, t) | (t, Type::Error) => t.clone(),
            (Type::Null, Type::Element) | (Type::Element, Type::Null) => Type::Element,
            (Type::Null, t) | (t, Type::Null) => self.nullable(t.clone(), span),
            (Type::StrLits(x), Type::StrLits(y)) => Type::str_lits(x.iter().chain(y.iter()).cloned().collect()),
            (Type::StrLits(_), Type::String) | (Type::String, Type::StrLits(_)) => Type::String,
            (Type::Nullable(x), t) | (t, Type::Nullable(x)) if self.conversion(t, x).is_some() => Type::Nullable(x.clone()),
            _ if self.conversion(b, a).is_some() => a.clone(),
            _ if self.conversion(a, b).is_some() => b.clone(),
            _ => {
                let msg = format!("the branches have different types: `{}` and `{}`", self.show(a), self.show(b));
                self.err(code::TYPE_MISMATCH, span, msg);
                Type::Error
            }
        }
    }

    fn ident(&mut self, name: &str, span: Span) -> TExpr {
        match self.lookup(name) {
            Some(Binding::Var(v)) => self.var_read(v, span),
            Some(Binding::Func(f)) => {
                let ft = self.func_type(f, span);
                TExpr::new(TExprKind::Closure(f), Type::Func(ft), span)
            }
            Some(Binding::Std(_)) | Some(Binding::StdObj(_)) => {
                self.err(code::UNSUPPORTED, span, format!("`{name}` can only be called, not used as a value"));
                TExpr::new(TExprKind::Null, Type::Error, span)
            }
            Some(Binding::Generic(_)) => {
                self.err_help(
                    code::GENERIC_USER,
                    span,
                    format!("`{name}` is generic; it can only be called directly"),
                    format!("write `{name}(...)` or `{name}<T>(...)`"),
                );
                TExpr::new(TExprKind::Null, Type::Error, span)
            }
            Some(Binding::Control(_)) => {
                self.err_help(code::UNSUPPORTED, span, format!("`{name}` is a control"), format!("write `<{name} ... />`"));
                TExpr::new(TExprKind::Null, Type::Error, span)
            }
            Some(Binding::Type(_)) | Some(Binding::Alias(..)) | Some(Binding::Enum(_)) => {
                self.err(code::UNKNOWN_NAME, span, format!("`{name}` is a type, not a value"));
                TExpr::new(TExprKind::Null, Type::Error, span)
            }
            None => {
                let help = match name {
                    "document" | "window" | "fetch" | "process" | "globalThis" | "setTimeout" | "localStorage" => {
                        "Plinth apps run in a closed world: use the `plinth:*` modules (SPEC.md §4.1)"
                    }
                    "console" | "Math" => "import it from \"plinth:core\"",
                    "signal" | "computed" | "effect" | "navigate" => "import it from \"plinth:ui\"",
                    _ => "",
                };
                let msg = format!("cannot find name `{name}`");
                if help.is_empty() {
                    self.err(code::UNKNOWN_NAME, span, msg);
                } else {
                    self.err_help(code::UNKNOWN_NAME, span, msg, help);
                }
                TExpr::new(TExprKind::Null, Type::Error, span)
            }
        }
    }

    fn member(&mut self, obj: &Expr, prop: &str, prop_span: Span, optional: bool, span: Span) -> TExpr {
        // `Enum.Member`, `Math.PI`.
        if let ExprKind::Ident(name) = &obj.kind {
            match self.lookup(name) {
                Some(Binding::Enum(id)) => {
                    let def = &self.prog.enums[id as usize];
                    return match def.members.iter().find(|(n, _)| n == prop) {
                        Some((_, v)) => TExpr::new(TExprKind::Num(*v as f64), Type::Enum(id), span),
                        None => {
                            let msg = format!("enum `{}` has no member `{prop}`", def.name);
                            self.err(code::NO_PROPERTY, prop_span, msg);
                            TExpr::new(TExprKind::Num(0.0), Type::Error, span)
                        }
                    };
                }
                Some(Binding::StdObj(StdObj::Math)) => {
                    let v = match prop {
                        "PI" => std::f64::consts::PI,
                        "E" => std::f64::consts::E,
                        _ => {
                            self.err(code::NO_PROPERTY, prop_span, format!("`Math.{prop}` is a function or does not exist"));
                            0.0
                        }
                    };
                    return TExpr::new(TExprKind::Num(v), Type::Number, span);
                }
                _ => {}
            }
        }
        let o = self.expr(obj, None);
        if optional {
            // `o?.prop` → `tmp = o; tmp === null ? null : tmp.prop`
            let inner = match &o.ty {
                Type::Nullable(t) => (**t).clone(),
                t => t.clone(),
            };
            let tmp = self.temp(o.ty.clone());
            let read = TExpr::new(TExprKind::Var(tmp), o.ty.clone(), o.span);
            let unwrapped = if inner == Type::Number {
                TExpr::new(TExprKind::Coerce(Coercion::UnboxNum, bx(read.clone())), inner.clone(), o.span)
            } else {
                TExpr::new(TExprKind::Coerce(Coercion::Retag, bx(read.clone())), inner.clone(), o.span)
            };
            let access = self.property(unwrapped, prop, prop_span, span);
            let result_ty = self.nullable(access.ty.clone(), span);
            let access = self.coerce(access, &result_ty);
            let null = TExpr::new(TExprKind::Null, Type::Null, span);
            let null = self.coerce(null, &result_ty);
            let test = TExpr::new(TExprKind::IsNull(bx(read)), Type::Bool, span);
            let cond = TExpr::new(TExprKind::Cond(bx(test), bx(null), bx(access)), result_ty.clone(), span);
            return TExpr::new(TExprKind::Block(vec![TStmt::Let(tmp, Some(o))], bx(cond)), result_ty, span);
        }
        self.property(o, prop, prop_span, span)
    }

    /// A non-method property read: struct fields and `length`.
    fn property(&mut self, o: TExpr, prop: &str, prop_span: Span, span: Span) -> TExpr {
        match o.ty.clone() {
            Type::Struct(sid) => match self.prog.structs[sid as usize].field(prop).map(|(i, f)| (i, f.ty.clone())) {
                Some((idx, ty)) => TExpr::new(TExprKind::Field(bx(o), sid, idx as u32), ty, span),
                None => {
                    let s = &self.prog.structs[sid as usize];
                    let names: Vec<&str> = s.fields.iter().map(|f| f.name.as_str()).collect();
                    let msg = format!("`{}` has no field `{prop}`", s.name);
                    let help = format!("the fields are: {}", names.join(", "));
                    self.err_help(code::NO_PROPERTY, prop_span, msg, help);
                    TExpr::new(TExprKind::Null, Type::Error, span)
                }
            },
            Type::Array(_) if prop == "length" => {
                let len = TExpr::new(TExprKind::Rt("arr_len", vec![o]), Type::Bool, span);
                TExpr::new(TExprKind::Coerce(Coercion::I32ToNum, bx(len)), Type::Number, span)
            }
            Type::String | Type::StrLits(_) if prop == "length" => TExpr::new(TExprKind::Rt("str_len", vec![o]), Type::Number, span),
            Type::Map(k, v) if prop == "size" => {
                let sid = self.map_struct(&k, &v);
                self.arr_len_of(self.kv_field(&o, sid, 0, span), span)
            }
            Type::Set(t) if prop == "size" => {
                let sid = self.set_struct(&t);
                self.arr_len_of(self.kv_field(&o, sid, 0, span), span)
            }
            Type::Map(..) | Type::Set(_) => {
                self.err_help(
                    code::NO_PROPERTY,
                    prop_span,
                    format!("`{}` has no property `{prop}`", self.show(&o.ty)),
                    "use `.size`, or a method like `.get`/`.has`",
                );
                TExpr::new(TExprKind::Null, Type::Error, span)
            }
            Type::Nullable(_) => {
                let msg = format!("the value may be null (type `{}`)", self.show(&o.ty));
                self.err_help(code::NULLABLE, o.span, msg, "check for null first, or use `?.`");
                TExpr::new(TExprKind::Null, Type::Error, span)
            }
            Type::Union(members) => match self.union_discriminant(&members) {
                Some((name, lits)) if name == prop => {
                    let ty = Type::str_lits(lits.into_iter().map(|(l, _)| l).collect());
                    TExpr::new(TExprKind::UnionTag(bx(o)), ty, span)
                }
                Some((name, _)) => {
                    let msg = format!("`{}` has no property `{prop}`", self.show(&o.ty));
                    self.err_help(code::NO_PROPERTY, prop_span, msg, format!("narrow it first, or read `.{name}`"));
                    TExpr::new(TExprKind::Null, Type::Error, span)
                }
                None => {
                    let msg = format!("`{}` has no property `{prop}`", self.show(&o.ty));
                    self.err_help(code::NO_PROPERTY, prop_span, msg, "narrow the union first (with `typeof` or a shared literal field)");
                    TExpr::new(TExprKind::Null, Type::Error, span)
                }
            },
            Type::Signal(_) | Type::Computed(_) => {
                self.err_help(
                    code::NO_PROPERTY,
                    prop_span,
                    format!("a signal has no property `{prop}`"),
                    "call the signal to read its value: `s().field`",
                );
                TExpr::new(TExprKind::Null, Type::Error, span)
            }
            Type::Error => TExpr::new(TExprKind::Null, Type::Error, span),
            other => {
                let msg = format!("`{}` has no property `{prop}`", self.show(&other));
                self.err(code::NO_PROPERTY, prop_span, msg);
                TExpr::new(TExprKind::Null, Type::Error, span)
            }
        }
    }

    // -- Calls ------------------------------------------------------------

    fn call(&mut self, callee: &Expr, type_args: &[ast::TypeAnn], args: &[Expr], span: Span, expected: Option<&Type>) -> TExpr {
        match &callee.kind {
            ExprKind::Ident(name) => match self.lookup(name) {
                Some(Binding::Std(f)) => return self.std_call(f, type_args, args, span, expected),
                Some(Binding::Func(fid)) => {
                    let ft = self.func_type(fid, callee.span);
                    let targs = self.call_args(&ft, args, span);
                    return TExpr::new(TExprKind::Call(fid, targs), ft.ret.clone(), span);
                }
                Some(Binding::Generic(gid)) => {
                    return match self.instantiate_generic(gid, type_args, args, span) {
                        Some((fid, ft)) => {
                            let targs = self.call_args(&ft, args, span);
                            TExpr::new(TExprKind::Call(fid, targs), ft.ret.clone(), span)
                        }
                        None => TExpr::new(TExprKind::Null, Type::Error, span),
                    };
                }
                _ => {}
            },
            ExprKind::Member { obj, prop, prop_span, optional: false } => {
                if let ExprKind::Ident(name) = &obj.kind
                    && let Some(Binding::StdObj(o)) = self.lookup(name)
                {
                    return self.std_obj_call(o, prop, *prop_span, args, span);
                }
                let o = self.expr(obj, None);
                return self.method_call(o, prop, *prop_span, args, span);
            }
            _ => {}
        }
        if !type_args.is_empty() {
            self.err(code::GENERIC_USER, span, "type arguments are only allowed on `signal` and `computed`");
        }
        let c = self.expr(callee, None);
        match c.ty.clone() {
            Type::Signal(t) => {
                self.no_args(args, span);
                self.check_reactive_read(span);
                TExpr::new(TExprKind::SignalGet(bx(c)), *t, span)
            }
            Type::Computed(t) => {
                self.no_args(args, span);
                self.check_reactive_read(span);
                TExpr::new(TExprKind::ComputedGet(bx(c)), *t, span)
            }
            Type::Func(ft) => {
                let targs = self.call_args(&ft, args, span);
                TExpr::new(TExprKind::CallClosure(bx(c), targs), ft.ret.clone(), span)
            }
            Type::Error => {
                for a in args {
                    self.expr(a, None);
                }
                TExpr::new(TExprKind::Null, Type::Error, span)
            }
            other => {
                let msg = format!("a value of type `{}` cannot be called", self.show(&other));
                self.err(code::NOT_CALLABLE, callee.span, msg);
                TExpr::new(TExprKind::Null, Type::Error, span)
            }
        }
    }

    pub(super) fn no_args(&mut self, args: &[Expr], span: Span) {
        if !args.is_empty() {
            self.err(code::ARG_COUNT, span, "this call takes no arguments");
        }
    }

    /// Checks arguments against a signature. Missing optional arguments
    /// become null.
    pub(super) fn call_args(&mut self, ft: &FuncType, args: &[Expr], span: Span) -> Vec<TExpr> {
        if args.len() < ft.required || args.len() > ft.params.len() {
            let msg = if ft.required == ft.params.len() {
                format!("expected {} argument(s), got {}", ft.params.len(), args.len())
            } else {
                format!("expected {} to {} arguments, got {}", ft.required, ft.params.len(), args.len())
            };
            self.err(code::ARG_COUNT, span, msg);
        }
        let mut out = Vec::new();
        for (i, p) in ft.params.iter().enumerate() {
            match args.get(i) {
                Some(a) => {
                    let te = self.expr_with(a, p);
                    out.push(self.coerce(te, p));
                }
                None => {
                    let null = TExpr::new(TExprKind::Null, Type::Null, span);
                    out.push(self.coerce(null, p));
                }
            }
        }
        out
    }

    /// Checks an expression with an expected type; closures get contextual
    /// parameter types from a function type.
    pub(super) fn expr_with(&mut self, e: &Expr, expected: &Type) -> TExpr {
        if let (ExprKind::Func(f), Type::Func(ft)) = (&e.kind, expected) {
            let ft = ft.clone();
            return self.closure(f, Some(&ft), "<closure>");
        }
        self.expr(e, Some(expected))
    }

    /// Checks a callback argument for an array method or a std function.
    /// `params` are the parameter types the callee offers; the callback may
    /// take fewer. Returns the closure and its arity.
    pub(super) fn callback(&mut self, e: &Expr, params: &[Type], ret: Option<Type>) -> (TExpr, usize) {
        let offered = FuncType { params: params.to_vec(), required: 0, ret: ret.clone().unwrap_or(Type::Error) };
        let te = match &e.kind {
            ExprKind::Func(f) => {
                if f.params.len() > params.len() {
                    self.err(code::ARG_COUNT, f.span, format!("this callback gets at most {} argument(s)", params.len()));
                }
                let mut ft = offered.clone();
                ft.params.truncate(f.params.len());
                self.closure(f, Some(&ft), "<callback>")
            }
            _ => self.expr(e, None),
        };
        match te.ty.clone() {
            Type::Func(ft) => {
                let n = ft.params.len();
                if n > params.len() {
                    self.err(code::ARG_COUNT, e.span, format!("this callback gets at most {} argument(s)", params.len()));
                    return (te, 0);
                }
                for (have, want) in ft.params.iter().zip(params) {
                    if have.repr() != want.repr() || self.conversion(want, have).is_none() {
                        let msg = format!("the callback parameter must accept `{}`", self.show(want));
                        self.err(code::TYPE_MISMATCH, e.span, msg);
                    }
                }
                if let Some(r) = &ret
                    && *r != Type::Void
                    && !ft.ret.is_error()
                    && (ft.ret.repr() != r.repr() || self.conversion(&ft.ret, r).is_none())
                {
                    let msg = format!("the callback must return `{}`, not `{}`", self.show(r), self.show(&ft.ret));
                    self.err(code::TYPE_MISMATCH, e.span, msg);
                }
                (te, n)
            }
            Type::Error => (te, 0),
            other => {
                let msg = format!("expected a function, not `{}`", self.show(&other));
                self.err(code::TYPE_MISMATCH, e.span, msg);
                (TExpr { ty: Type::Error, ..te }, 0)
            }
        }
    }

    fn method_call(&mut self, o: TExpr, prop: &str, prop_span: Span, args: &[Expr], span: Span) -> TExpr {
        match o.ty.clone() {
            Type::Signal(t) => match prop {
                "set" => {
                    if args.len() != 1 {
                        self.err(code::ARG_COUNT, span, "`set` takes one argument");
                        return TExpr::new(TExprKind::Null, Type::Void, span);
                    }
                    // The argument runs once, synchronously, right here (like
                    // `update`'s callback): a signal read in it is the normal
                    // "read current value(s), compute the next one" pattern,
                    // not the "not reactive" bug, so do not lint it.
                    let saved = self.fx.reactive;
                    if saved == ReactiveCtx::Plain {
                        self.fx.reactive = ReactiveCtx::Callback;
                    }
                    let v = self.expr_with(&args[0], &t);
                    self.fx.reactive = saved;
                    let v = self.coerce(v, &t);
                    TExpr::new(TExprKind::SignalSet(bx(o), bx(v)), Type::Void, span)
                }
                "update" => {
                    if args.len() != 1 {
                        self.err(code::ARG_COUNT, span, "`update` takes one function");
                        return TExpr::new(TExprKind::Null, Type::Void, span);
                    }
                    let (f, arity) = self.callback(&args[0], std::slice::from_ref(&*t), Some((*t).clone()));
                    // tmp = signal; tmp.set(f(peek(tmp)))
                    let tmp = self.temp(o.ty.clone());
                    let read = TExpr::new(TExprKind::Var(tmp), o.ty.clone(), o.span);
                    let peek = TExpr::new(TExprKind::SignalPeek(bx(read.clone())), (*t).clone(), span);
                    let call_args = if arity == 1 { vec![peek] } else { Vec::new() };
                    let ret = match &f.ty {
                        Type::Func(ft) => ft.ret.clone(),
                        _ => Type::Error,
                    };
                    let call = TExpr::new(TExprKind::CallClosure(bx(f), call_args), ret, span);
                    let call = self.coerce(call, &t);
                    let set = TExpr::new(TExprKind::SignalSet(bx(read), bx(call)), Type::Void, span);
                    TExpr::new(TExprKind::Block(vec![TStmt::Let(tmp, Some(o))], bx(set)), Type::Void, span)
                }
                _ => {
                    self.err_help(code::NO_PROPERTY, prop_span, format!("a signal has no method `{prop}`"), "use `set` or `update`");
                    TExpr::new(TExprKind::Null, Type::Error, span)
                }
            },
            Type::String | Type::StrLits(_) => self.string_method(o, prop, prop_span, args, span),
            Type::Number => match prop {
                "toFixed" => {
                    let d = match args.first() {
                        Some(a) => {
                            let te = self.expr(a, Some(&Type::Number));
                            self.coerce(te, &Type::Number)
                        }
                        None => TExpr::new(TExprKind::Num(0.0), Type::Number, span),
                    };
                    TExpr::new(TExprKind::Rt("f64_to_fixed", vec![o, d]), Type::String, span)
                }
                _ => {
                    self.err(code::NO_PROPERTY, prop_span, format!("`number` has no method `{prop}`"));
                    TExpr::new(TExprKind::Null, Type::Error, span)
                }
            },
            Type::Array(elem) => self.array_method(o, (*elem).clone(), prop, prop_span, args, span),
            Type::Map(k, v) => self.map_method(o, &k, &v, prop, prop_span, args, span),
            Type::Set(t) => self.set_method(o, &t, prop, prop_span, args, span),
            Type::Struct(_) => {
                // A function-typed field: `obj.f(x)`.
                let field = self.property(o, prop, prop_span, span);
                match field.ty.clone() {
                    Type::Func(ft) => {
                        let targs = self.call_args(&ft, args, span);
                        TExpr::new(TExprKind::CallClosure(bx(field), targs), ft.ret.clone(), span)
                    }
                    Type::Error => field,
                    other => {
                        let msg = format!("the field `{prop}` has type `{}` and cannot be called", self.show(&other));
                        self.err(code::NOT_CALLABLE, prop_span, msg);
                        TExpr::new(TExprKind::Null, Type::Error, span)
                    }
                }
            }
            Type::Error => {
                for a in args {
                    self.expr(a, None);
                }
                TExpr::new(TExprKind::Null, Type::Error, span)
            }
            other => {
                let msg = format!("`{}` has no method `{prop}`", self.show(&other));
                let help = if let Type::Nullable(_) = other { "check for null first" } else { "" };
                if help.is_empty() {
                    self.err(code::NO_PROPERTY, prop_span, msg);
                } else {
                    self.err_help(code::NULLABLE, prop_span, msg, help);
                }
                TExpr::new(TExprKind::Null, Type::Error, span)
            }
        }
    }

    fn str_arg(&mut self, args: &[Expr], i: usize, span: Span) -> TExpr {
        match args.get(i) {
            Some(a) => {
                let te = self.expr(a, Some(&Type::String));
                self.coerce(te, &Type::String)
            }
            None => {
                self.err(code::ARG_COUNT, span, "missing argument");
                TExpr::new(TExprKind::Str(String::new()), Type::String, span)
            }
        }
    }

    fn num_arg(&mut self, args: &[Expr], i: usize, default: f64, span: Span) -> TExpr {
        match args.get(i) {
            Some(a) => {
                let te = self.expr(a, Some(&Type::Number));
                self.coerce(te, &Type::Number)
            }
            None => TExpr::new(TExprKind::Num(default), Type::Number, span),
        }
    }

    fn string_method(&mut self, o: TExpr, prop: &str, prop_span: Span, args: &[Expr], span: Span) -> TExpr {
        let rt = |name: &'static str, args: Vec<TExpr>, ty: Type| TExpr::new(TExprKind::Rt(name, args), ty, span);
        let b = |e: TExpr| TExpr::new(TExprKind::Coerce(Coercion::Retag, bx(e)), Type::Bool, span);
        match prop {
            "trim" => rt("str_trim", vec![o], Type::String),
            "toUpperCase" => rt("str_upper", vec![o], Type::String),
            "toLowerCase" => rt("str_lower", vec![o], Type::String),
            "includes" => b(rt("str_includes", vec![o, self.str_arg(args, 0, span)], Type::Bool)),
            "startsWith" => b(rt("str_starts", vec![o, self.str_arg(args, 0, span)], Type::Bool)),
            "endsWith" => b(rt("str_ends", vec![o, self.str_arg(args, 0, span)], Type::Bool)),
            "indexOf" => rt("str_index_of", vec![o, self.str_arg(args, 0, span)], Type::Number),
            "slice" => {
                let a = self.num_arg(args, 0, 0.0, span);
                let e = self.num_arg(args, 1, f64::NAN, span);
                rt("str_slice", vec![o, a, e], Type::String)
            }
            "repeat" => {
                let n = self.num_arg(args, 0, 0.0, span);
                rt("str_repeat", vec![o, n], Type::String)
            }
            _ => {
                self.err(code::NO_PROPERTY, prop_span, format!("`string` has no method `{prop}` in Plinth TS"));
                TExpr::new(TExprKind::Null, Type::Error, span)
            }
        }
    }

    fn array_method(&mut self, o: TExpr, elem: Type, prop: &str, prop_span: Span, args: &[Expr], span: Span) -> TExpr {
        let hof = |c: &mut Self, kind: ArrayHof, o: TExpr, ret: Option<Type>, result: Type| {
            if args.len() != 1 {
                c.err(code::ARG_COUNT, span, format!("`{prop}` takes one function"));
                return TExpr::new(TExprKind::Null, Type::Error, span);
            }
            let (f, arity) = c.callback(&args[0], &[elem.clone(), Type::Number], ret);
            let result = if kind == ArrayHof::Map {
                match &f.ty {
                    Type::Func(ft) => Type::Array(Box::new(c.widen(ft.ret.clone()))),
                    _ => Type::Error,
                }
            } else {
                result
            };
            if kind == ArrayHof::Map
                && let Type::Array(e) = &result
                && **e == Type::Void
            {
                c.err_help(code::TYPE_MISMATCH, span, "the `map` callback returns nothing", "use `forEach`");
            }
            TExpr::new(TExprKind::ArrayHof { kind, arr: bx(o), f: bx(f), arity }, result, span)
        };
        let arr_ty = o.ty.clone();
        match prop {
            "map" => hof(self, ArrayHof::Map, o, None, Type::Error),
            "filter" => hof(self, ArrayHof::Filter, o, Some(Type::Bool), arr_ty),
            "find" => {
                let r = self.nullable(elem.clone(), span);
                hof(self, ArrayHof::Find, o, Some(Type::Bool), r)
            }
            "findIndex" => hof(self, ArrayHof::FindIndex, o, Some(Type::Bool), Type::Number),
            "some" => hof(self, ArrayHof::Some, o, Some(Type::Bool), Type::Bool),
            "every" => hof(self, ArrayHof::Every, o, Some(Type::Bool), Type::Bool),
            "forEach" => hof(self, ArrayHof::ForEach, o, Some(Type::Void), Type::Void),
            "includes" | "indexOf" => {
                let v = match args.first() {
                    Some(a) => {
                        let te = self.expr(a, Some(&elem));
                        self.coerce(te, &elem)
                    }
                    None => {
                        self.err(code::ARG_COUNT, span, format!("`{prop}` takes one value"));
                        return TExpr::new(TExprKind::Null, Type::Error, span);
                    }
                };
                let eq = match self.widen(elem.clone()) {
                    Type::Number => EqKind::F64,
                    Type::Bool | Type::Enum(_) | Type::Element => EqKind::I32,
                    Type::String => EqKind::Str,
                    _ => EqKind::Ref,
                };
                let index = prop == "indexOf";
                TExpr::new(
                    TExprKind::ArraySearch { index, eq, arr: bx(o), value: bx(v) },
                    if index { Type::Number } else { Type::Bool },
                    span,
                )
            }
            "push" => {
                let tmp = self.temp(arr_ty.clone());
                let read = TExpr::new(TExprKind::Var(tmp), arr_ty.clone(), o.span);
                let mut stmts = vec![TStmt::Let(tmp, Some(o))];
                let mut last = None;
                for a in args {
                    let te = self.expr_with(a, &elem);
                    let te = self.coerce(te, &elem);
                    let f = if elem.repr() == crate::types::Repr::F64 { "arr_push_f64" } else { "arr_push_i32" };
                    if let Some(prev) = last.take() {
                        stmts.push(TStmt::Expr(TExpr::new(TExprKind::Coerce(Coercion::Discard, bx(prev)), Type::Void, span)));
                    }
                    last = Some(TExpr::new(TExprKind::Rt(f, vec![read.clone(), te]), Type::Bool, span));
                }
                let len = last.unwrap_or_else(|| TExpr::new(TExprKind::Rt("arr_len", vec![read]), Type::Bool, span));
                let len = TExpr::new(TExprKind::Coerce(Coercion::I32ToNum, bx(len)), Type::Number, span);
                TExpr::new(TExprKind::Block(stmts, bx(len)), Type::Number, span)
            }
            "pop" => {
                self.no_args(args, span);
                let r = self.nullable(elem.clone(), span);
                if elem.repr() == crate::types::Repr::F64 {
                    let v = TExpr::new(TExprKind::Rt("arr_pop_f64", vec![o]), Type::Number, span);
                    TExpr::new(TExprKind::Coerce(Coercion::BoxNum, bx(v)), r, span)
                } else {
                    TExpr::new(TExprKind::Rt("arr_pop_i32", vec![o]), r, span)
                }
            }
            "slice" => {
                let a = self.num_arg(args, 0, 0.0, span);
                let e = self.num_arg(args, 1, i32::MAX as f64, span);
                let a = TExpr::new(TExprKind::Coerce(Coercion::NumToI32, bx(a)), Type::Bool, span);
                let e = TExpr::new(TExprKind::Coerce(Coercion::NumToI32, bx(e)), Type::Bool, span);
                TExpr::new(TExprKind::Rt("arr_slice", vec![o, a, e]), arr_ty, span)
            }
            "reverse" => {
                self.no_args(args, span);
                let tmp = self.temp(arr_ty.clone());
                let read = TExpr::new(TExprKind::Var(tmp), arr_ty.clone(), o.span);
                let rev = TExpr::new(TExprKind::Rt("arr_reverse", vec![read.clone()]), Type::Void, span);
                TExpr::new(TExprKind::Block(vec![TStmt::Let(tmp, Some(o)), TStmt::Expr(rev)], bx(read)), arr_ty, span)
            }
            "join" => {
                if !elem.is_stringish() && !elem.is_error() {
                    self.err_help(code::TYPE_MISMATCH, span, "`join` works on string arrays", "map the items to strings first");
                }
                let sep = match args.first() {
                    Some(_) => self.str_arg(args, 0, span),
                    None => TExpr::new(TExprKind::Str(",".into()), Type::String, span),
                };
                TExpr::new(TExprKind::Rt("arr_join", vec![o, sep]), Type::String, span)
            }
            _ => {
                self.err(code::NO_PROPERTY, prop_span, format!("arrays have no method `{prop}` in Plinth TS"));
                TExpr::new(TExprKind::Null, Type::Error, span)
            }
        }
    }

    // -- `Map` / `Set` (HANDOFF.md item 5) ---------------------------------
    //
    // Both are the internal struct `{ keys: K[] }` (`Set`) or
    // `{ keys: K[], values: V[] }` (`Map`), built in `Checker::map_struct`/
    // `set_struct`. Every method desugars to array operations the checker
    // already has (`ArraySearch`, `Index`, `Assign(Place::Index, ..)`,
    // `arr_push_*`/`arr_pop_*`), so none of this needs new codegen or a new
    // `plinth-rt` module. Lookup, insert and delete are linear scans: fine
    // for the small, form-sized state Plinth apps keep (SPEC.md §4.2).

    /// A value that produces nothing on the stack, for the tail of a
    /// `Type::Void` block (unlike `TExprKind::Null`, which always pushes a
    /// zero and is only safe where the surrounding statement drops it).
    fn void_tail(&self, span: Span) -> TExpr {
        let dummy = TExpr::new(TExprKind::Bool(true), Type::Bool, span);
        TExpr::new(TExprKind::Coerce(Coercion::Discard, bx(dummy)), Type::Void, span)
    }

    /// Field `idx` of a `Map`/`Set` object (its `keys` or `values` array).
    fn kv_field(&self, obj: &TExpr, sid: crate::types::StructId, idx: u32, span: Span) -> TExpr {
        let ty = self.prog.structs[sid as usize].fields[idx as usize].ty.clone();
        TExpr::new(TExprKind::Field(bx(obj.clone()), sid, idx), ty, span)
    }

    fn arr_len_of(&self, arr: TExpr, span: Span) -> TExpr {
        let len = TExpr::new(TExprKind::Rt("arr_len", vec![arr]), Type::Bool, span);
        TExpr::new(TExprKind::Coerce(Coercion::I32ToNum, bx(len)), Type::Number, span)
    }

    fn arr_get_at(&self, arr: TExpr, idx: TExpr, elem_ty: Type, span: Span) -> TExpr {
        TExpr::new(TExprKind::Index(bx(arr), bx(idx)), elem_ty, span)
    }

    fn arr_set_at(&self, arr: TExpr, idx: TExpr, value: TExpr, span: Span) -> TStmt {
        let elem_ty = value.ty.clone();
        TStmt::Expr(TExpr::new(TExprKind::Assign(Place::Index(bx(arr), bx(idx)), bx(value)), elem_ty, span))
    }

    fn arr_push_discard(&self, arr: TExpr, value: TExpr, span: Span) -> TStmt {
        let f = if value.ty.repr() == crate::types::Repr::F64 { "arr_push_f64" } else { "arr_push_i32" };
        let push = TExpr::new(TExprKind::Rt(f, vec![arr, value]), Type::Bool, span);
        TStmt::Expr(TExpr::new(TExprKind::Coerce(Coercion::Discard, bx(push)), Type::Void, span))
    }

    fn arr_pop_discard(&self, arr: TExpr, span: Span) -> TStmt {
        let elem_ty = match &arr.ty {
            Type::Array(e) => (**e).clone(),
            _ => unreachable!(),
        };
        let f = if elem_ty.repr() == crate::types::Repr::F64 { "arr_pop_f64" } else { "arr_pop_i32" };
        let pop = TExpr::new(TExprKind::Rt(f, vec![arr]), elem_ty, span);
        TStmt::Expr(TExpr::new(TExprKind::Coerce(Coercion::Discard, bx(pop)), Type::Void, span))
    }

    /// `while (arr.length > 0) arr.pop();`
    fn arr_clear(&mut self, arr: TExpr, span: Span) -> Vec<TStmt> {
        let arr_v = self.temp(arr.ty.clone());
        let arr_r = TExpr::new(TExprKind::Var(arr_v), arr.ty.clone(), span);
        let cond = TExpr::new(
            TExprKind::Cmp(CmpOp::Gt, EqKind::F64, bx(self.arr_len_of(arr_r.clone(), span)), bx(TExpr::new(TExprKind::Num(0.0), Type::Number, span))),
            Type::Bool,
            span,
        );
        let id = self.prog.new_loop();
        let pop = self.arr_pop_discard(arr_r, span);
        vec![TStmt::Let(arr_v, Some(arr)), TStmt::Loop { id, cond: Some(cond), test_after: false, update: None, body: vec![pop] }]
    }

    /// `m.delete(k)` / `s.delete(v)`: finds the key, then swaps it with the
    /// last entry and pops (so deletion does not keep insertion order).
    fn kv_delete(&mut self, sid: crate::types::StructId, has_values: bool, eq: EqKind, o: TExpr, key: TExpr, span: Span) -> TExpr {
        let obj_v = self.temp(o.ty.clone());
        let obj_r = TExpr::new(TExprKind::Var(obj_v), o.ty.clone(), span);
        let keys = self.kv_field(&obj_r, sid, 0, span);
        let idx_v = self.temp(Type::Number);
        let idx_r = TExpr::new(TExprKind::Var(idx_v), Type::Number, span);
        let search = TExpr::new(TExprKind::ArraySearch { index: true, eq, arr: bx(keys.clone()), value: bx(key) }, Type::Number, span);
        let not_found = TExpr::new(
            TExprKind::Cmp(CmpOp::Eq, EqKind::F64, bx(idx_r.clone()), bx(TExpr::new(TExprKind::Num(-1.0), Type::Number, span))),
            Type::Bool,
            span,
        );
        let last_v = self.temp(Type::Number);
        let last_r = TExpr::new(TExprKind::Var(last_v), Type::Number, span);
        let last_init = TExpr::new(
            TExprKind::Num2(NumOp::Sub, bx(self.arr_len_of(keys.clone(), span)), bx(TExpr::new(TExprKind::Num(1.0), Type::Number, span))),
            Type::Number,
            span,
        );
        let idx_ne_last = TExpr::new(TExprKind::Cmp(CmpOp::Ne, EqKind::F64, bx(idx_r.clone()), bx(last_r.clone())), Type::Bool, span);
        let keys_elem = match &keys.ty {
            Type::Array(e) => (**e).clone(),
            _ => unreachable!(),
        };
        let mut swap_body = vec![self.arr_set_at(keys.clone(), idx_r.clone(), self.arr_get_at(keys.clone(), last_r.clone(), keys_elem, span), span)];
        if has_values {
            let values = self.kv_field(&obj_r, sid, 1, span);
            let values_elem = match &values.ty {
                Type::Array(e) => (**e).clone(),
                _ => unreachable!(),
            };
            swap_body.push(self.arr_set_at(values.clone(), idx_r.clone(), self.arr_get_at(values, last_r.clone(), values_elem, span), span));
        }
        let mut pop_stmts = vec![TStmt::If(idx_ne_last, swap_body, Vec::new()), self.arr_pop_discard(keys, span)];
        if has_values {
            pop_stmts.push(self.arr_pop_discard(self.kv_field(&obj_r, sid, 1, span), span));
        }
        let found_block =
            TExpr::new(TExprKind::Block(pop_stmts, bx(TExpr::new(TExprKind::Bool(true), Type::Bool, span))), Type::Bool, span);
        let cond =
            TExpr::new(TExprKind::Cond(bx(not_found), bx(TExpr::new(TExprKind::Bool(false), Type::Bool, span)), bx(found_block)), Type::Bool, span);
        let prelude = vec![TStmt::Let(obj_v, Some(o)), TStmt::Let(idx_v, Some(search)), TStmt::Let(last_v, Some(last_init))];
        TExpr::new(TExprKind::Block(prelude, bx(cond)), Type::Bool, span)
    }

    fn map_method(&mut self, o: TExpr, k: &Type, v: &Type, prop: &str, prop_span: Span, args: &[Expr], span: Span) -> TExpr {
        let sid = self.map_struct(k, v);
        let eq = self.key_eq(k);
        let wrong_arity = |c: &mut Self, what: &str, n: usize| {
            c.err(code::ARG_COUNT, span, format!("`{what}` takes {n} argument(s)"));
        };
        match prop {
            "get" => {
                if args.len() != 1 {
                    wrong_arity(self, "get", 1);
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let key = self.expr_with(&args[0], k);
                let key = self.coerce(key, k);
                let obj_v = self.temp(o.ty.clone());
                let obj_r = TExpr::new(TExprKind::Var(obj_v), o.ty.clone(), span);
                let keys = self.kv_field(&obj_r, sid, 0, span);
                let values = self.kv_field(&obj_r, sid, 1, span);
                let idx_v = self.temp(Type::Number);
                let idx_r = TExpr::new(TExprKind::Var(idx_v), Type::Number, span);
                let search = TExpr::new(TExprKind::ArraySearch { index: true, eq, arr: bx(keys), value: bx(key) }, Type::Number, span);
                let not_found = TExpr::new(
                    TExprKind::Cmp(CmpOp::Eq, EqKind::F64, bx(idx_r.clone()), bx(TExpr::new(TExprKind::Num(-1.0), Type::Number, span))),
                    Type::Bool,
                    span,
                );
                let result_ty = self.nullable(v.clone(), span);
                let null_v = self.coerce(TExpr::new(TExprKind::Null, Type::Null, span), &result_ty);
                let found = self.coerce(self.arr_get_at(values, idx_r, v.clone(), span), &result_ty);
                let cond = TExpr::new(TExprKind::Cond(bx(not_found), bx(null_v), bx(found)), result_ty.clone(), span);
                TExpr::new(TExprKind::Block(vec![TStmt::Let(obj_v, Some(o)), TStmt::Let(idx_v, Some(search))], bx(cond)), result_ty, span)
            }
            "set" => {
                if args.len() != 2 {
                    wrong_arity(self, "set", 2);
                    return self.void_tail(span);
                }
                let key = self.expr_with(&args[0], k);
                let key = self.coerce(key, k);
                let val = self.expr_with(&args[1], v);
                let val = self.coerce(val, v);
                let obj_v = self.temp(o.ty.clone());
                let obj_r = TExpr::new(TExprKind::Var(obj_v), o.ty.clone(), span);
                let keys = self.kv_field(&obj_r, sid, 0, span);
                let values = self.kv_field(&obj_r, sid, 1, span);
                let key_v = self.temp(k.clone());
                let key_r = TExpr::new(TExprKind::Var(key_v), k.clone(), span);
                let idx_v = self.temp(Type::Number);
                let idx_r = TExpr::new(TExprKind::Var(idx_v), Type::Number, span);
                let search = TExpr::new(TExprKind::ArraySearch { index: true, eq, arr: bx(keys.clone()), value: bx(key_r.clone()) }, Type::Number, span);
                let found = TExpr::new(
                    TExprKind::Cmp(CmpOp::Ne, EqKind::F64, bx(idx_r.clone()), bx(TExpr::new(TExprKind::Num(-1.0), Type::Number, span))),
                    Type::Bool,
                    span,
                );
                let set_existing = self.arr_set_at(values.clone(), idx_r, val.clone(), span);
                let push_key = self.arr_push_discard(keys, key_r, span);
                let push_val = self.arr_push_discard(values, val, span);
                let if_stmt = TStmt::If(found, vec![set_existing], vec![push_key, push_val]);
                let body = vec![TStmt::Let(obj_v, Some(o)), TStmt::Let(key_v, Some(key)), TStmt::Let(idx_v, Some(search)), if_stmt];
                TExpr::new(TExprKind::Block(body, bx(self.void_tail(span))), Type::Void, span)
            }
            "has" => {
                if args.len() != 1 {
                    wrong_arity(self, "has", 1);
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let key = self.expr_with(&args[0], k);
                let key = self.coerce(key, k);
                let keys = self.kv_field(&o, sid, 0, span);
                TExpr::new(TExprKind::ArraySearch { index: false, eq, arr: bx(keys), value: bx(key) }, Type::Bool, span)
            }
            "delete" => {
                if args.len() != 1 {
                    wrong_arity(self, "delete", 1);
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let key = self.expr_with(&args[0], k);
                let key = self.coerce(key, k);
                self.kv_delete(sid, true, eq, o, key, span)
            }
            "clear" => {
                self.no_args(args, span);
                let obj_v = self.temp(o.ty.clone());
                let obj_r = TExpr::new(TExprKind::Var(obj_v), o.ty.clone(), span);
                let mut body = vec![TStmt::Let(obj_v, Some(o))];
                body.extend(self.arr_clear(self.kv_field(&obj_r, sid, 0, span), span));
                body.extend(self.arr_clear(self.kv_field(&obj_r, sid, 1, span), span));
                TExpr::new(TExprKind::Block(body, bx(self.void_tail(span))), Type::Void, span)
            }
            _ => {
                self.err_help(code::NO_PROPERTY, prop_span, format!("`Map` has no method `{prop}`"), "use `get`, `set`, `has`, `delete` or `clear`");
                TExpr::new(TExprKind::Null, Type::Error, span)
            }
        }
    }

    fn set_method(&mut self, o: TExpr, t: &Type, prop: &str, prop_span: Span, args: &[Expr], span: Span) -> TExpr {
        let sid = self.set_struct(t);
        let eq = self.key_eq(t);
        match prop {
            "add" => {
                if args.len() != 1 {
                    self.err(code::ARG_COUNT, span, "`add` takes one value");
                    return self.void_tail(span);
                }
                let val = self.expr_with(&args[0], t);
                let val = self.coerce(val, t);
                let obj_v = self.temp(o.ty.clone());
                let obj_r = TExpr::new(TExprKind::Var(obj_v), o.ty.clone(), span);
                let keys = self.kv_field(&obj_r, sid, 0, span);
                let val_v = self.temp(t.clone());
                let val_r = TExpr::new(TExprKind::Var(val_v), t.clone(), span);
                let has = TExpr::new(TExprKind::ArraySearch { index: false, eq, arr: bx(keys.clone()), value: bx(val_r.clone()) }, Type::Bool, span);
                let not_has = TExpr::new(TExprKind::Not(bx(has)), Type::Bool, span);
                let push = self.arr_push_discard(keys, val_r, span);
                let if_stmt = TStmt::If(not_has, vec![push], Vec::new());
                let body = vec![TStmt::Let(obj_v, Some(o)), TStmt::Let(val_v, Some(val)), if_stmt];
                TExpr::new(TExprKind::Block(body, bx(self.void_tail(span))), Type::Void, span)
            }
            "has" => {
                if args.len() != 1 {
                    self.err(code::ARG_COUNT, span, "`has` takes one value");
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let val = self.expr_with(&args[0], t);
                let val = self.coerce(val, t);
                let keys = self.kv_field(&o, sid, 0, span);
                TExpr::new(TExprKind::ArraySearch { index: false, eq, arr: bx(keys), value: bx(val) }, Type::Bool, span)
            }
            "delete" => {
                if args.len() != 1 {
                    self.err(code::ARG_COUNT, span, "`delete` takes one value");
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let val = self.expr_with(&args[0], t);
                let val = self.coerce(val, t);
                self.kv_delete(sid, false, eq, o, val, span)
            }
            "clear" => {
                self.no_args(args, span);
                let obj_v = self.temp(o.ty.clone());
                let obj_r = TExpr::new(TExprKind::Var(obj_v), o.ty.clone(), span);
                let mut body = vec![TStmt::Let(obj_v, Some(o))];
                body.extend(self.arr_clear(self.kv_field(&obj_r, sid, 0, span), span));
                TExpr::new(TExprKind::Block(body, bx(self.void_tail(span))), Type::Void, span)
            }
            _ => {
                self.err_help(code::NO_PROPERTY, prop_span, format!("`Set` has no method `{prop}`"), "use `add`, `has`, `delete` or `clear`");
                TExpr::new(TExprKind::Null, Type::Error, span)
            }
        }
    }

    // -- Literals ---------------------------------------------------------

    fn array_lit(&mut self, elems: &[(bool, Expr)], expected: Option<&Type>, span: Span) -> TExpr {
        let mut elem_ty = match expected {
            Some(Type::Array(t)) => Some((**t).clone()),
            Some(Type::Nullable(inner)) => match &**inner {
                Type::Array(t) => Some((**t).clone()),
                _ => None,
            },
            _ => None,
        };
        let mut out = Vec::new();
        for (spread, e) in elems {
            let want = elem_ty.clone().map(|t| if *spread { Type::Array(Box::new(t)) } else { t });
            let te = match &want {
                Some(w) => self.expr_with(e, w),
                None => self.expr(e, None),
            };
            if elem_ty.is_none() {
                elem_ty = Some(match (&te.ty, spread) {
                    (Type::Array(t), true) => (**t).clone(),
                    (t, false) => self.widen(t.clone()),
                    (other, true) => {
                        let msg = format!("cannot spread a value of type `{}`", self.show(other));
                        self.err(code::TYPE_MISMATCH, e.span, msg);
                        Type::Error
                    }
                });
            }
            let t = elem_ty.clone().unwrap();
            let te = if *spread { self.coerce(te, &Type::Array(Box::new(t))) } else { self.coerce(te, &t) };
            out.push((*spread, te));
        }
        let Some(elem) = elem_ty else {
            self.err_help(code::CANNOT_INFER, span, "the element type of `[]` is unknown", "add a type: `const xs: T[] = []`");
            return TExpr::new(TExprKind::ArrayLit(out), Type::Error, span);
        };
        if elem == Type::Null {
            self.err(code::CANNOT_INFER, span, "an array of only `null` needs a type");
        }
        TExpr::new(TExprKind::ArrayLit(out), Type::Array(Box::new(elem)), span)
    }

    fn object_lit(&mut self, props: &[ObjProp], expected: Option<&Type>, span: Span) -> TExpr {
        // A union target: pick the member whose discriminant literal
        // matches this literal's discriminant field (an object union,
        // HANDOFF.md item 2).
        let union_target = |c: &Self, members: &[Type]| -> Option<crate::types::StructId> {
            let (name, lits) = c.union_discriminant(members)?;
            let found = props.iter().find_map(|p| match p {
                ObjProp::Field(n, e, _) if *n == name => match &e.kind {
                    ExprKind::Str(s) => Some(s.clone()),
                    _ => None,
                },
                _ => None,
            })?;
            let (_, idx) = lits.into_iter().find(|(l, _)| *l == found)?;
            match &members[idx] {
                Type::Struct(s) => Some(*s),
                _ => None,
            }
        };
        let target = match expected {
            Some(Type::Struct(s)) => Some(*s),
            Some(Type::Nullable(inner)) => match &**inner {
                Type::Struct(s) => Some(*s),
                Type::Union(members) => union_target(self, members),
                _ => None,
            },
            Some(Type::Union(members)) => union_target(self, members),
            _ => None,
        };
        // Evaluate the parts in source order into temporaries. `source`
        // records which part gives each field: the last one in source order.
        enum Src {
            Explicit(usize),
            Spread(usize),
        }
        let mut stmts = Vec::new();
        let mut explicit: Vec<(String, TExpr, Span)> = Vec::new();
        let mut spreads: Vec<TExpr> = Vec::new();
        let mut source: std::collections::HashMap<String, Src> = std::collections::HashMap::new();
        for p in props {
            match p {
                ObjProp::Field(name, e, fspan) => {
                    let want = target.and_then(|s| self.prog.structs[s as usize].field(name).map(|(_, f)| f.ty.clone()));
                    let te = match &want {
                        Some(w) => self.expr_with(e, w),
                        None => self.expr(e, None),
                    };
                    let te = match &want {
                        Some(w) => self.coerce(te, w),
                        None => te,
                    };
                    if explicit.iter().any(|(n, _, _)| n == name) {
                        self.err(code::DUPLICATE, *fspan, format!("duplicate field `{name}`"));
                        continue;
                    }
                    source.insert(name.clone(), Src::Explicit(explicit.len()));
                    explicit.push((name.clone(), te, *fspan));
                }
                ObjProp::Spread(e) => {
                    let te = self.expr(e, target.map(Type::Struct).as_ref());
                    let Type::Struct(src_id) = te.ty else {
                        if !te.ty.is_error() {
                            let msg = format!("cannot spread a value of type `{}` into an object", self.show(&te.ty));
                            self.err(code::TYPE_MISMATCH, e.span, msg);
                        }
                        continue;
                    };
                    for f in &self.prog.structs[src_id as usize].fields {
                        source.insert(f.name.clone(), Src::Spread(spreads.len()));
                    }
                    let tmp = self.temp(te.ty.clone());
                    let ty = te.ty.clone();
                    stmts.push(TStmt::Let(tmp, Some(te)));
                    spreads.push(TExpr::new(TExprKind::Var(tmp), ty, e.span));
                }
            }
        }
        // The layout: the expected struct, the first spread's struct, or a
        // new anonymous struct from the explicit fields.
        let first_spread = spreads.first().and_then(|s| if let Type::Struct(id) = s.ty { Some(id) } else { None });
        let sid = match target.or(first_spread) {
            Some(s) => s,
            None => {
                let fields: Vec<Field> = explicit
                    .iter()
                    .map(|(n, e, _)| Field { name: n.clone(), ty: self.widen(e.ty.clone()), optional: false })
                    .collect();
                if fields.iter().any(|f| f.ty == Type::Null) {
                    self.err_help(code::CANNOT_INFER, span, "a field with only `null` needs a type", "declare an interface for the object");
                }
                match self.anon_struct(fields) {
                    Type::Struct(s) => s,
                    _ => unreachable!(),
                }
            }
        };
        let def = self.prog.structs[sid as usize].clone();
        for (n, _, fspan) in &explicit {
            if def.field(n).is_none() {
                let names: Vec<&str> = def.fields.iter().map(|f| f.name.as_str()).collect();
                let msg = format!("`{}` has no field `{n}`", def.name);
                self.err_help(code::NO_PROPERTY, *fspan, msg, format!("the fields are: {}", names.join(", ")));
            }
        }
        let mut values = Vec::new();
        for f in &def.fields {
            let v = match source.get(&f.name) {
                Some(Src::Explicit(i)) => {
                    let e = explicit[*i].1.clone();
                    self.coerce(e, &f.ty)
                }
                Some(Src::Spread(k)) => {
                    let s = spreads[*k].clone();
                    let Type::Struct(src_id) = s.ty else { unreachable!() };
                    let (idx, sf) = self.prog.structs[src_id as usize].field(&f.name).map(|(i, f)| (i, f.ty.clone())).unwrap();
                    let read = TExpr::new(TExprKind::Field(bx(s), src_id, idx as u32), sf, span);
                    self.coerce(read, &f.ty)
                }
                None if f.optional || matches!(f.ty, Type::Nullable(_)) => {
                    let null = TExpr::new(TExprKind::Null, Type::Null, span);
                    self.coerce(null, &f.ty)
                }
                None => {
                    let msg = format!("missing field `{}` of `{}`", f.name, def.name);
                    self.err(code::MISSING_FIELD, span, msg);
                    TExpr::new(TExprKind::Null, Type::Error, span)
                }
            };
            values.push(v);
        }
        let lit = TExpr::new(TExprKind::StructLit(sid, values), Type::Struct(sid), span);
        if stmts.is_empty() { lit } else { TExpr::new(TExprKind::Block(stmts, bx(lit)), Type::Struct(sid), span) }
    }

    // -- Operators --------------------------------------------------------

    fn binary(&mut self, op: BinOp, l: &Expr, r: &Expr, span: Span) -> TExpr {
        match op {
            BinOp::Add => {
                let lt = self.expr(l, None);
                let rt = self.expr(r, None);
                if lt.ty.is_stringish() || rt.ty.is_stringish() {
                    let a = self.to_str(lt);
                    let b = self.to_str(rt);
                    return TExpr::new(TExprKind::Concat(bx(a), bx(b)), Type::String, span);
                }
                if lt.ty == Type::Int && rt.ty == Type::Int {
                    return TExpr::new(TExprKind::Int2(IntOp::Add, bx(lt), bx(rt)), Type::Int, span);
                }
                let a = self.coerce(lt, &Type::Number);
                let b = self.coerce(rt, &Type::Number);
                TExpr::new(TExprKind::Num2(NumOp::Add, bx(a), bx(b)), Type::Number, span)
            }
            BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem | BinOp::Pow => {
                let lt = self.expr(l, None);
                let rt = self.expr(r, None);
                if op != BinOp::Pow && lt.ty == Type::Int && rt.ty == Type::Int {
                    let iop = match op {
                        BinOp::Sub => IntOp::Sub,
                        BinOp::Mul => IntOp::Mul,
                        BinOp::Div => IntOp::Div,
                        _ => IntOp::Rem,
                    };
                    return TExpr::new(TExprKind::Int2(iop, bx(lt), bx(rt)), Type::Int, span);
                }
                let a = self.coerce(lt, &Type::Number);
                let b = self.coerce(rt, &Type::Number);
                let nop = match op {
                    BinOp::Sub => NumOp::Sub,
                    BinOp::Mul => NumOp::Mul,
                    BinOp::Div => NumOp::Div,
                    BinOp::Rem => NumOp::Rem,
                    _ => NumOp::Pow,
                };
                TExpr::new(TExprKind::Num2(nop, bx(a), bx(b)), Type::Number, span)
            }
            BinOp::Eq | BinOp::Ne => {
                if let Some(test) = self.typeof_narrow(l, r, span) {
                    return if op == BinOp::Ne { TExpr::new(TExprKind::Not(bx(test)), Type::Bool, span) } else { test };
                }
                let lt = self.expr(l, None);
                let rt = self.expr(r, Some(&lt.ty.clone()));
                let test = self.equality(lt, rt, span);
                if op == BinOp::Ne { TExpr::new(TExprKind::Not(bx(test)), Type::Bool, span) } else { test }
            }
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                let cop = match op {
                    BinOp::Lt => CmpOp::Lt,
                    BinOp::Le => CmpOp::Le,
                    BinOp::Gt => CmpOp::Gt,
                    _ => CmpOp::Ge,
                };
                let lt = self.expr(l, None);
                let rt = self.expr(r, None);
                if lt.ty.is_stringish() && rt.ty.is_stringish() {
                    return TExpr::new(TExprKind::StrCmp(cop, bx(lt), bx(rt)), Type::Bool, span);
                }
                let a = self.coerce(lt, &Type::Number);
                let b = self.coerce(rt, &Type::Number);
                TExpr::new(TExprKind::Cmp(cop, EqKind::F64, bx(a), bx(b)), Type::Bool, span)
            }
        }
    }

    /// `a === b`.
    /// `typeof x === "string"` or `"string" === typeof x`: narrows a union
    /// to its `string` member (or, for `"object"`, to its non-string
    /// members). `None` if this is not that pattern.
    fn typeof_narrow(&mut self, l: &Expr, r: &Expr, span: Span) -> Option<TExpr> {
        let (inner, cat, cat_span) = match (&l.kind, &r.kind) {
            (ExprKind::Unary(UnOp::Typeof, inner), ExprKind::Str(cat)) => (inner, cat, r.span),
            (ExprKind::Str(cat), ExprKind::Unary(UnOp::Typeof, inner)) => (inner, cat, l.span),
            _ => return None,
        };
        let te = self.expr(inner, None);
        let Type::Union(members) = &te.ty else {
            self.err_help(
                code::ADVANCED_TYPE,
                inner.span,
                format!("`typeof` narrowing needs a union type, not `{}`", self.show(&te.ty)),
                "give the value a union type, e.g. `let x: Shape | string`",
            );
            return Some(TExpr::new(TExprKind::Bool(false), Type::Error, span));
        };
        let idxs: Vec<usize> = match cat.as_str() {
            "string" => members.iter().enumerate().filter(|(_, m)| m.is_stringish()).map(|(i, _)| i).collect(),
            "object" => members.iter().enumerate().filter(|(_, m)| matches!(m, Type::Struct(_))).map(|(i, _)| i).collect(),
            _ => {
                self.err_help(code::ADVANCED_TYPE, cat_span, format!("`typeof` narrowing to {cat:?} is not supported yet"), "use \"string\" or \"object\"");
                Vec::new()
            }
        };
        if idxs.is_empty() {
            let msg = format!("`{}` has no `{cat}` member", self.show(&te.ty));
            self.err(code::TYPE_MISMATCH, cat_span, msg);
            return Some(TExpr::new(TExprKind::Bool(false), Type::Error, span));
        }
        Some(TExpr::new(TExprKind::UnionIs(Box::new(te), idxs), Type::Bool, span))
    }

    /// `union.tag === "lit"` or `"lit" === union.tag`: narrows the union
    /// to the member whose discriminant literal is `lit`.
    fn discriminant_narrow(&mut self, a: &TExpr, b: &TExpr, span: Span) -> Option<TExpr> {
        let (tag, lit) = match (&a.kind, &b.kind) {
            (TExprKind::UnionTag(o), TExprKind::Str(s)) => (o, s),
            (TExprKind::Str(s), TExprKind::UnionTag(o)) => (o, s),
            _ => return None,
        };
        let Type::Union(members) = &tag.ty else { return None };
        let (_, lits) = self.union_discriminant(members)?;
        let idx = lits.into_iter().find(|(l, _)| l == lit)?.1;
        Some(TExpr::new(TExprKind::UnionIs(Box::new((**tag).clone()), vec![idx]), Type::Bool, span))
    }

    fn equality(&mut self, a: TExpr, b: TExpr, span: Span) -> TExpr {
        if let Some(t) = self.discriminant_narrow(&a, &b, span) {
            return t;
        }
        let is_null = |t: &TExpr| matches!(t.kind, TExprKind::Null);
        if is_null(&a) || is_null(&b) {
            let other = if is_null(&a) { b } else { a };
            if !matches!(other.ty.repr(), crate::types::Repr::Ref) && !other.ty.is_error() && other.ty != Type::Element {
                let msg = format!("a value of type `{}` is never null", self.show(&other.ty));
                self.err(code::TYPE_MISMATCH, span, msg);
            }
            return TExpr::new(TExprKind::IsNull(bx(other)), Type::Bool, span);
        }
        let (lt, rt) = (self.widen(a.ty.clone()), self.widen(b.ty.clone()));
        // `T | null` with `T`: false when the nullable side is null.
        let null_kind = |c: &Self, n: &Type, t: &Type| -> Option<EqKind> {
            let Type::Nullable(inner) = n else { return None };
            match (c.widen((**inner).clone()), t) {
                (Type::String, Type::String) => Some(EqKind::NullStr),
                (Type::Number, Type::Number) => Some(EqKind::NullF64),
                (x, y) if x == *y && x.repr() == crate::types::Repr::Ref => Some(EqKind::Ref),
                _ => None,
            }
        };
        if let Some(k) = null_kind(self, &lt, &rt) {
            return TExpr::new(TExprKind::Cmp(CmpOp::Eq, k, bx(a), bx(b)), Type::Bool, span);
        }
        if let Some(k) = null_kind(self, &rt, &lt) {
            // The nullable side must come first; keep the evaluation order
            // with a temporary for the left side.
            let tmp = self.temp(a.ty.clone());
            let read = TExpr::new(TExprKind::Var(tmp), a.ty.clone(), a.span);
            let cmp = TExpr::new(TExprKind::Cmp(CmpOp::Eq, k, bx(b), bx(read)), Type::Bool, span);
            return TExpr::new(TExprKind::Block(vec![TStmt::Let(tmp, Some(a))], bx(cmp)), Type::Bool, span);
        }
        let eq = match (&lt, &rt) {
            (Type::Number, Type::Number) => EqKind::F64,
            (Type::Bool, Type::Bool) | (Type::Element, Type::Element) => EqKind::I32,
            (Type::Enum(x), Type::Enum(y)) if x == y => EqKind::I32,
            (Type::Int, Type::Int) => EqKind::I32,
            (Type::String, Type::String) => EqKind::Str,
            (Type::Error, _) | (_, Type::Error) => EqKind::I32,
            (x, y) if x == y && x.repr() == crate::types::Repr::Ref => EqKind::Ref,
            _ => {
                let msg = format!("cannot compare `{}` with `{}`", self.show(&a.ty), self.show(&b.ty));
                let help = if matches!(lt, Type::Nullable(_)) || matches!(rt, Type::Nullable(_)) {
                    "check for null first"
                } else {
                    "the two sides must have the same type"
                };
                self.err_help(code::TYPE_MISMATCH, span, msg, help);
                EqKind::I32
            }
        };
        TExpr::new(TExprKind::Cmp(CmpOp::Eq, eq, bx(a), bx(b)), Type::Bool, span)
    }

    fn logical(&mut self, op: LogicOp, l: &Expr, r: &Expr, span: Span, expected: Option<&Type>) -> TExpr {
        match op {
            LogicOp::Nullish => {
                let lt = self.expr(l, None);
                let inner = match &lt.ty {
                    Type::Nullable(t) => (**t).clone(),
                    t => {
                        if !t.is_error() {
                            let msg = format!("the left side has type `{}`, which is never null", self.show(t));
                            self.err(code::TYPE_MISMATCH, l.span, msg);
                        }
                        t.clone()
                    }
                };
                let want = expected.cloned().unwrap_or(inner.clone());
                let rt = self.expr(r, Some(&want));
                let ty = self.join(&inner, &rt.ty, span);
                let tmp = self.temp(lt.ty.clone());
                let read = TExpr::new(TExprKind::Var(tmp), lt.ty.clone(), l.span);
                let unwrapped = if inner == Type::Number && matches!(lt.ty, Type::Nullable(_)) {
                    TExpr::new(TExprKind::Coerce(Coercion::UnboxNum, bx(read.clone())), Type::Number, l.span)
                } else {
                    TExpr::new(TExprKind::Coerce(Coercion::Retag, bx(read.clone())), inner, l.span)
                };
                let unwrapped = self.coerce(unwrapped, &ty);
                let rt = self.coerce(rt, &ty);
                let test = TExpr::new(TExprKind::IsNull(bx(read)), Type::Bool, span);
                let cond = TExpr::new(TExprKind::Cond(bx(test), bx(rt), bx(unwrapped)), ty.clone(), span);
                TExpr::new(TExprKind::Block(vec![TStmt::Let(tmp, Some(lt))], bx(cond)), ty, span)
            }
            LogicOp::And | LogicOp::Or => {
                let lt = self.expr(l, None);
                let lt_t = self.truthy(lt.clone());
                // The right side sees the narrowing of the left side.
                let (when_true, when_false) = self.narrowing(&lt_t);
                self.push_scope();
                self.fx.scopes.last_mut().unwrap().narrow.extend(if op == LogicOp::And { when_true } else { when_false });
                let rt = self.expr(r, expected);
                self.pop_scope();
                // `cond && <X/>`: an element or nothing.
                if op == LogicOp::And && rt.ty == Type::Element {
                    let none = TExpr::new(TExprKind::Null, Type::Element, span);
                    return TExpr::new(TExprKind::Cond(bx(lt_t), bx(rt), bx(none)), Type::Element, span);
                }
                if lt.ty == Type::Bool && rt.ty == Type::Bool || lt.ty.is_error() || rt.ty.is_error() {
                    let rt = self.truthy(rt);
                    let kind = if op == LogicOp::And { TExprKind::And(bx(lt_t), bx(rt)) } else { TExprKind::Or(bx(lt_t), bx(rt)) };
                    return TExpr::new(kind, Type::Bool, span);
                }
                if op == LogicOp::Or && self.conversion(&rt.ty, &self.widen(lt.ty.clone())).is_some() {
                    // `a || b` with one type: `tmp = a; truthy(tmp) ? tmp : b`.
                    let ty = self.widen(lt.ty.clone());
                    let tmp = self.temp(ty.clone());
                    let read = TExpr::new(TExprKind::Var(tmp), ty.clone(), l.span);
                    let test = self.truthy(read.clone());
                    let rt = self.coerce(rt, &ty);
                    let lt = self.coerce(lt, &ty);
                    let cond = TExpr::new(TExprKind::Cond(bx(test), bx(read), bx(rt)), ty.clone(), span);
                    return TExpr::new(TExprKind::Block(vec![TStmt::Let(tmp, Some(lt))], bx(cond)), ty, span);
                }
                self.err_help(
                    code::TYPE_MISMATCH,
                    span,
                    format!("`{}` works on booleans here", if op == LogicOp::And { "&&" } else { "||" }),
                    "use a ternary (`a ? b : c`) or `??`",
                );
                TExpr::new(TExprKind::Bool(false), Type::Error, span)
            }
        }
    }

    // -- Assignment -------------------------------------------------------

    fn place(&mut self, target: &Expr) -> Option<(Place, Type)> {
        match &target.kind {
            ExprKind::Ident(name) => match self.lookup(name) {
                Some(Binding::Var(v)) => {
                    let info = &self.prog.vars[v as usize];
                    let is_param = self.prog.funcs[info.owner as usize].params.contains(&v);
                    if !info.mutable && !is_param {
                        let msg = format!("cannot assign to `{name}`; it is a `const`");
                        self.err_help(code::ASSIGN_CONST, target.span, msg, "declare it with `let`");
                    }
                    Some((Place::Var(v), self.prog.vars[v as usize].ty.clone()))
                }
                Some(Binding::Func(_)) => {
                    self.err(code::ASSIGN_CONST, target.span, format!("cannot assign to the function `{name}`"));
                    None
                }
                _ => {
                    self.err(code::UNKNOWN_NAME, target.span, format!("cannot find name `{name}`"));
                    None
                }
            },
            ExprKind::Member { obj, prop, prop_span, optional: false } => {
                let o = self.expr(obj, None);
                match o.ty.clone() {
                    Type::Struct(sid) => match self.prog.structs[sid as usize].field(prop).map(|(i, f)| (i, f.ty.clone())) {
                        Some((idx, ty)) => Some((Place::Field(bx(o), sid, idx as u32), ty)),
                        None => {
                            let msg = format!("`{}` has no field `{prop}`", self.prog.structs[sid as usize].name);
                            self.err(code::NO_PROPERTY, *prop_span, msg);
                            None
                        }
                    },
                    Type::Error => None,
                    other => {
                        let msg = format!("cannot assign to a property of `{}`", self.show(&other));
                        self.err(code::NO_PROPERTY, *prop_span, msg);
                        None
                    }
                }
            }
            ExprKind::Index { obj, index, optional: false } => {
                let o = self.expr(obj, None);
                match o.ty.clone() {
                    Type::Array(elem) => {
                        let i = self.expr(index, Some(&Type::Number));
                        let i = self.coerce(i, &Type::Number);
                        Some((Place::Index(bx(o), bx(i)), *elem))
                    }
                    Type::Error => None,
                    other => {
                        let msg = format!("cannot index a value of type `{}`", self.show(&other));
                        self.err(code::COMPUTED_ACCESS, target.span, msg);
                        None
                    }
                }
            }
            _ => {
                self.err(code::UNSUPPORTED, target.span, "this assignment target is not supported");
                None
            }
        }
    }

    /// Makes the place safe to read and write: the object and index go into
    /// temporaries.
    fn stable_place(&mut self, place: Place, stmts: &mut Vec<TStmt>) -> Place {
        match place {
            Place::Var(v) => Place::Var(v),
            Place::Field(o, sid, idx) => {
                let tmp = self.temp(o.ty.clone());
                let ty = o.ty.clone();
                let span = o.span;
                stmts.push(TStmt::Let(tmp, Some(*o)));
                Place::Field(bx(TExpr::new(TExprKind::Var(tmp), ty, span)), sid, idx)
            }
            Place::Index(a, i) => {
                let (ta, ti) = (self.temp(a.ty.clone()), self.temp(Type::Number));
                let (aty, span) = (a.ty.clone(), a.span);
                stmts.push(TStmt::Let(ta, Some(*a)));
                stmts.push(TStmt::Let(ti, Some(*i)));
                Place::Index(
                    bx(TExpr::new(TExprKind::Var(ta), aty, span)),
                    bx(TExpr::new(TExprKind::Var(ti), Type::Number, span)),
                )
            }
        }
    }

    fn read_place(&self, place: &Place, ty: &Type, span: Span) -> TExpr {
        let kind = match place {
            Place::Var(v) => TExprKind::Var(*v),
            Place::Field(o, sid, idx) => TExprKind::Field(o.clone(), *sid, *idx),
            Place::Index(a, i) => TExprKind::Index(a.clone(), i.clone()),
        };
        TExpr::new(kind, ty.clone(), span)
    }

    fn assign(&mut self, op: Option<BinOp>, target: &Expr, value: &Expr, span: Span) -> TExpr {
        let Some((place, ty)) = self.place(target) else {
            self.expr(value, None);
            return TExpr::new(TExprKind::Null, Type::Error, span);
        };
        match op {
            None => {
                let v = self.expr_with(value, &ty);
                let v = self.coerce(v, &ty);
                TExpr::new(TExprKind::Assign(place, bx(v)), ty, span)
            }
            Some(op) => {
                let mut stmts = Vec::new();
                let place = self.stable_place(place, &mut stmts);
                let cur = self.read_place(&place, &ty, target.span);
                let rhs = self.expr(value, None);
                let combined = if op == BinOp::Add && ty.is_stringish() {
                    let r = self.to_str(rhs);
                    TExpr::new(TExprKind::Concat(bx(cur), bx(r)), Type::String, span)
                } else {
                    let nop = match op {
                        BinOp::Add => NumOp::Add,
                        BinOp::Sub => NumOp::Sub,
                        BinOp::Mul => NumOp::Mul,
                        BinOp::Div => NumOp::Div,
                        BinOp::Rem => NumOp::Rem,
                        _ => NumOp::Pow,
                    };
                    let a = self.coerce(cur, &Type::Number);
                    let b = self.coerce(rhs, &Type::Number);
                    TExpr::new(TExprKind::Num2(nop, bx(a), bx(b)), Type::Number, span)
                };
                let combined = self.coerce(combined, &ty);
                let assign = TExpr::new(TExprKind::Assign(place, bx(combined)), ty.clone(), span);
                if stmts.is_empty() { assign } else { TExpr::new(TExprKind::Block(stmts, bx(assign)), ty, span) }
            }
        }
    }

    fn update(&mut self, inc: bool, prefix: bool, target: &Expr, span: Span) -> TExpr {
        let Some((place, ty)) = self.place(target) else {
            return TExpr::new(TExprKind::Null, Type::Error, span);
        };
        if ty != Type::Number && !ty.is_error() {
            let msg = format!("`++` and `--` work on numbers, not `{}`", self.show(&ty));
            self.err(code::TYPE_MISMATCH, target.span, msg);
        }
        let mut stmts = Vec::new();
        let place = self.stable_place(place, &mut stmts);
        let old = self.temp(Type::Number);
        let cur = self.read_place(&place, &Type::Number, target.span);
        stmts.push(TStmt::Let(old, Some(cur)));
        let old_read = TExpr::new(TExprKind::Var(old), Type::Number, span);
        let one = TExpr::new(TExprKind::Num(1.0), Type::Number, span);
        let new = TExpr::new(
            TExprKind::Num2(if inc { NumOp::Add } else { NumOp::Sub }, bx(old_read.clone()), bx(one)),
            Type::Number,
            span,
        );
        let assign = TExpr::new(TExprKind::Assign(place, bx(new)), Type::Number, span);
        if prefix {
            TExpr::new(TExprKind::Block(stmts, bx(assign)), Type::Number, span)
        } else {
            stmts.push(TStmt::Expr(assign));
            TExpr::new(TExprKind::Block(stmts, bx(old_read)), Type::Number, span)
        }
    }

    /// The function type of a closure expression.
    pub(super) fn func_ty(&self, e: &TExpr) -> Option<Rc<FuncType>> {
        match &e.ty {
            Type::Func(ft) => Some(ft.clone()),
            _ => None,
        }
    }
}
