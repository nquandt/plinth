//! Expressions.

use super::{Binding, Checker, ReactiveCtx, StdFn, StdObj};
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
                    Type::Struct(sid) if self.tuple_shape(sid).is_some() => match self.tuple_index(sid, index) {
                        Some(TupleAt::Field(i, ty)) => TExpr::new(TExprKind::Field(bx(o), sid, i), ty, span),
                        Some(TupleAt::Rest(f, k, ty)) => {
                            let arr = TExpr::new(TExprKind::Field(bx(o), sid, f), Type::Array(Box::new(ty.clone())), span);
                            TExpr::new(TExprKind::Index(bx(arr), bx(self.num_lit(k as f64, span))), ty, span)
                        }
                        None => TExpr::new(TExprKind::Null, Type::Error, span),
                    },
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
            ExprKind::NewInstance(name, args) => self.new_instance(name, args, span),
            ExprKind::NewPromise(type_args, args) => self.new_promise_expr(type_args, args, expected, span),
            ExprKind::InstanceOf(obj, name, name_span) => self.instance_of(obj, name, *name_span, span),
            ExprKind::As(inner, type_ann, cast_span) => self.as_cast(inner, type_ann, *cast_span),
            ExprKind::Await(inner) => {
                if !self.fx.is_async {
                    self.err_help(code::ASYNC, span, "`await` is only allowed in an `async` function", "mark the function `async`");
                    self.expr(inner, None);
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let te = self.expr(inner, None);
                match self.promise_of(&te.ty) {
                    Some(info) => TExpr::new(TExprKind::Await(bx(te)), info.ty.clone(), span),
                    None => {
                        if !te.ty.is_error() {
                            let msg = format!("`await` needs a `Promise`, not `{}`", self.show(&te.ty));
                            self.err(code::TYPE_MISMATCH, inner.span, msg);
                        }
                        TExpr::new(TExprKind::Null, Type::Error, span)
                    }
                }
            }
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
            Some(Binding::Type(_)) | Some(Binding::Alias(..)) | Some(Binding::Interface(..)) | Some(Binding::Enum(_)) => {
                self.err(code::UNKNOWN_NAME, span, format!("`{name}` is a type, not a value"));
                TExpr::new(TExprKind::Null, Type::Error, span)
            }
            // `Infinity` and `NaN` from `std/lib.d.ts`.
            None if name == "Infinity" => TExpr::new(TExprKind::Num(f64::INFINITY), Type::Number, span),
            None if name == "NaN" => TExpr::new(TExprKind::Num(f64::NAN), Type::Number, span),
            None if name == "this" => {
                self.err_help(
                    code::THIS,
                    span,
                    "`this` is not allowed outside a method or constructor",
                    "move this code into a method, or pass the value in as a parameter",
                );
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
            let unwrapped = match inner.repr() {
                crate::types::Repr::F64 => TExpr::new(TExprKind::Coerce(Coercion::UnboxNum, bx(read.clone())), inner.clone(), o.span),
                crate::types::Repr::I32 => TExpr::new(TExprKind::Coerce(Coercion::UnboxI32, bx(read.clone())), inner.clone(), o.span),
                _ => TExpr::new(TExprKind::Coerce(Coercion::Retag, bx(read.clone())), inner.clone(), o.span),
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
            Type::Struct(sid) if prop == "length" && self.tuple_shape(sid).is_some() => self.tuple_length(o, sid, span),
            Type::Struct(sid) => match self.prog.structs[sid as usize].field(prop).map(|(i, f)| (i, f.ty.clone())) {
                Some((idx, ty)) => {
                    // A member path rooted at a `const`/parameter (`r.subtitle`)
                    // is narrowed the same way a plain variable is (HANDOFF.md
                    // "Found later": narrowing on member expressions).
                    let base_path = self.narrow_path(&o);
                    let field = TExpr::new(TExprKind::Field(bx(o), sid, idx as u32), ty, span);
                    match base_path {
                        Some((v, mut path)) if path.len() < 2 => {
                            path.push(idx as u32);
                            self.apply_narrowing(field, (v, path))
                        }
                        _ => field,
                    }
                }
                None => {
                    if self.resolve_method(sid, prop).is_some() {
                        let msg = format!("`{prop}` is a method; it cannot be used without calling it");
                        self.err_help(code::UNBOUND_METHOD, prop_span, msg, format!("write `() => c.{prop}()`"));
                        return TExpr::new(TExprKind::Null, Type::Error, span);
                    }
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
        let r = self.call_impl(callee, type_args, args, span, expected);
        // Any call can mutate state reached through aliases of a narrowed
        // member path (`r.subtitle`), so a call conservatively drops all
        // member-path narrowing (docs/language.md "Narrowing"). Plain
        // variable narrowing is unaffected: it only ever applies to a
        // `const`/parameter, which a call cannot reassign.
        self.invalidate_member_narrowing();
        r
    }

    fn call_impl(&mut self, callee: &Expr, type_args: &[ast::TypeAnn], args: &[Expr], span: Span, expected: Option<&Type>) -> TExpr {
        match &callee.kind {
            ExprKind::Ident(name) if name == "super" => {
                // The one legitimate `super(...)` call is the first
                // statement of a subclass constructor, extracted and
                // checked directly by `declare_class`; any other one
                // reaches this generic path, which is always an error
                // (SPEC.md §4.2 v1).
                self.err_help(
                    code::SUPER,
                    callee.span,
                    "`super(...)` is only valid as the first statement of a subclass constructor",
                    "move it there; it can be called only once",
                );
                for a in args {
                    self.expr(a, None);
                }
                return TExpr::new(TExprKind::Null, Type::Error, span);
            }
            ExprKind::Ident(name) => match self.lookup(name) {
                Some(Binding::Std(f)) => return self.std_call(f, type_args, args, span, expected),
                Some(Binding::StdObj(StdObj::Navigate)) => return self.std_call(StdFn::Navigate, type_args, args, span, expected),
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
                if let ExprKind::Ident(name) = &obj.kind {
                    if name == "super" {
                        return self.super_method_call(prop, *prop_span, args, span);
                    }
                    if let Some(Binding::StdObj(o)) = self.lookup(name) {
                        return self.std_obj_call(o, prop, *prop_span, type_args, args, span);
                    }
                    if name == "Promise" && self.lookup(name).is_none() {
                        return self.promise_static(prop, *prop_span, type_args, args, span, expected);
                    }
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

    /// Checks the arguments of a method call (`c.m(a)`), with `this` as the
    /// lowered function's first argument (SPEC.md §4.2: static dispatch).
    fn method_args(&mut self, ft: &Rc<FuncType>, this: TExpr, args: &[Expr], span: Span) -> Vec<TExpr> {
        let this = self.coerce(this, &ft.params[0]);
        let rest = FuncType { params: ft.params[1..].to_vec(), required: ft.required.saturating_sub(1), ret: ft.ret.clone() };
        let mut out = vec![this];
        out.extend(self.call_args(&rest, args, span));
        out
    }

    /// `super.m(args)` (SPEC.md §4.2 v1): always a direct call to the base
    /// class's method, never a dynamic dispatch.
    fn super_method_call(&mut self, prop: &str, prop_span: Span, args: &[Expr], span: Span) -> TExpr {
        let Some(Binding::Var(this_var)) = self.lookup("this") else {
            self.err(code::SUPER, span, "`super` is only valid inside a class method or constructor");
            for a in args {
                self.expr(a, None);
            }
            return TExpr::new(TExprKind::Null, Type::Error, span);
        };
        let Type::Struct(sid) = self.prog.vars[this_var as usize].ty.clone() else {
            self.err(code::SUPER, span, "`super` is only valid inside a class method or constructor");
            return TExpr::new(TExprKind::Null, Type::Error, span);
        };
        let Some(base) = self.classes.get(&sid).and_then(|c| c.base) else {
            self.err_help(code::SUPER, span, "this class has no base class", "`super` only works in a subclass");
            return TExpr::new(TExprKind::Null, Type::Error, span);
        };
        let Some(fid) = self.resolve_method(base, prop) else {
            let msg = format!("the base class has no method `{prop}`");
            self.err(code::NO_PROPERTY, prop_span, msg);
            return TExpr::new(TExprKind::Null, Type::Error, span);
        };
        let ft = self.func_type(fid, prop_span);
        let this = TExpr::new(TExprKind::Var(this_var), Type::Struct(sid), span);
        let targs = self.method_args(&ft, this, args, span);
        TExpr::new(TExprKind::Call(fid, targs), ft.ret.clone(), span)
    }

    /// `new C(args)`: calls the class's lowered constructor function.
    fn new_instance(&mut self, name: &str, args: &[Expr], span: Span) -> TExpr {
        let sid = match self.lookup(name) {
            Some(Binding::Type(Type::Struct(sid))) if self.classes.contains_key(&sid) => sid,
            Some(Binding::Type(_)) => {
                self.err_help(code::CLASS, span, format!("`{name}` is not a class"), "`new` only works on a class");
                return TExpr::new(TExprKind::Null, Type::Error, span);
            }
            Some(_) => {
                self.err(code::NOT_CALLABLE, span, format!("`{name}` cannot be constructed with `new`"));
                return TExpr::new(TExprKind::Null, Type::Error, span);
            }
            None => {
                self.err(code::UNKNOWN_NAME, span, format!("cannot find name `{name}`"));
                return TExpr::new(TExprKind::Null, Type::Error, span);
            }
        };
        let Some(ctor) = self.classes[&sid].ctor else {
            // Already reported ("a class needs a constructor") when declared.
            for a in args {
                self.expr(a, None);
            }
            return TExpr::new(TExprKind::Null, Type::Error, span);
        };
        let ft = self.func_type(ctor, span);
        let targs = self.call_args(&ft, args, span);
        TExpr::new(TExprKind::Call(ctor, targs), Type::Struct(sid), span)
    }

    /// `x instanceof C` (SPEC.md §4.2 v1): true if `x`'s runtime type is `C`
    /// or one of its subclasses.
    fn instance_of(&mut self, obj: &Expr, name: &str, name_span: Span, span: Span) -> TExpr {
        let sid = match self.lookup(name) {
            Some(Binding::Type(Type::Struct(sid))) if self.classes.contains_key(&sid) => sid,
            Some(Binding::Type(_)) => {
                self.err_help(code::CLASS, name_span, format!("`{name}` is not a class"), "`instanceof` only works with a class");
                self.expr(obj, None);
                return TExpr::new(TExprKind::Null, Type::Error, span);
            }
            _ => {
                self.err(code::UNKNOWN_NAME, name_span, format!("cannot find name `{name}`"));
                self.expr(obj, None);
                return TExpr::new(TExprKind::Null, Type::Error, span);
            }
        };
        let o = self.expr(obj, None);
        match o.ty.clone() {
            Type::Struct(_) => TExpr::new(TExprKind::InstanceOf(bx(o), sid), Type::Bool, span),
            Type::Nullable(inner) if matches!(*inner, Type::Struct(_)) => {
                let tmp = self.temp(o.ty.clone());
                let read = TExpr::new(TExprKind::Var(tmp), o.ty.clone(), o.span);
                let is_null = TExpr::new(TExprKind::IsNull(bx(read.clone())), Type::Bool, span);
                let unwrapped = TExpr::new(TExprKind::Coerce(Coercion::Retag, bx(read)), (*inner).clone(), span);
                let test = TExpr::new(TExprKind::InstanceOf(bx(unwrapped), sid), Type::Bool, span);
                let false_lit = TExpr::new(TExprKind::Bool(false), Type::Bool, span);
                let cond = TExpr::new(TExprKind::Cond(bx(is_null), bx(false_lit), bx(test)), Type::Bool, span);
                TExpr::new(TExprKind::Block(vec![TStmt::Let(tmp, Some(o))], bx(cond)), Type::Bool, span)
            }
            Type::Error => TExpr::new(TExprKind::Null, Type::Error, span),
            other => {
                let msg = format!("`instanceof` needs a class value, not `{}`", self.show(&other));
                self.err(code::TYPE_MISMATCH, obj.span, msg);
                TExpr::new(TExprKind::Null, Type::Error, span)
            }
        }
    }

    /// `expr as T` (dogfooding gap #6). TypeScript's `as` is normally a
    /// compile-time-only annotation, but Plinth has no dynamic `any`
    /// escape hatch to fall back on if the annotation is wrong, so every
    /// cast this accepts carries its own run-time check that `throw`s
    /// (traps) if the value turns out not to be what was claimed. Two
    /// shapes are safe enough to support cheaply:
    ///
    /// 1. `string` (or a wider literal union) to a narrower string
    ///    literal union: checked by comparing the string against the
    ///    target's literal set, the same run-time check already used for
    ///    `JSON.parse<T>()` (`json_decode_str_lits`).
    /// 2. A discriminated union to one of its members: checked by
    ///    comparing the union's shared discriminant tag (`UnionTag`,
    ///    the same mechanism `.tag` property access uses) against the
    ///    target member's literal tag.
    ///
    /// `number as int` is deliberately NOT a cast (there is nothing to
    /// check at run time that would make it safe; `int(x)` truncates and
    /// is the sanctioned conversion). Everything else still gets PL2006.
    fn as_cast(&mut self, inner: &Expr, type_ann: &ast::TypeAnn, span: Span) -> TExpr {
        let target = self.resolve_type(type_ann);
        let te = self.expr(inner, None);
        let from = te.ty.clone();
        if from == target {
            return TExpr::new(te.kind, target, span);
        }
        match (&from, &target) {
            (Type::String, Type::StrLits(lits)) | (Type::StrLits(_), Type::StrLits(lits)) => {
                return self.cast_str_to_lits(te, lits.clone(), span);
            }
            _ => {}
        }
        if let Type::Union(members) = &from {
            if members.iter().any(|m| *m == target) {
                return self.cast_union_to_member(te, members.clone(), target, span);
            }
        }
        if from == Type::Number && target == Type::Int {
            self.err_help(code::TYPE_ASSERTION, span, "`as int` is not a cast", "use `int(x)` to convert and truncate instead");
            return TExpr::new(TExprKind::Null, Type::Error, span);
        }
        let msg = format!("cannot cast `{}` to `{}`: the types are unrelated", self.show(&from), self.show(&target));
        self.err_help(
            code::TYPE_ASSERTION,
            span,
            msg,
            "`as` only narrows `string` to a literal union, or a discriminated union to one of its members; for anything else, check the value at run time (a type guard function, `typeof`, or a discriminant field comparison) or annotate the variable type instead",
        );
        TExpr::new(TExprKind::Null, Type::Error, span)
    }

    /// `str as "a" | "b" | ...`: trap (via `throw`, like any other runtime
    /// error) if the string is not one of the target's literals.
    fn cast_str_to_lits(&mut self, te: TExpr, lits: Rc<[String]>, span: Span) -> TExpr {
        let v = self.temp(Type::String);
        let v_r = TExpr::new(TExprKind::Var(v), Type::String, span);
        let mut cond: Option<TExpr> = None;
        for lit in lits.iter() {
            let eq = TExpr::new(
                TExprKind::Cmp(CmpOp::Eq, EqKind::Str, bx(v_r.clone()), bx(TExpr::new(TExprKind::Str(lit.clone()), Type::String, span))),
                Type::Bool,
                span,
            );
            cond = Some(match cond {
                Some(c) => TExpr::new(TExprKind::Or(bx(c), bx(eq)), Type::Bool, span),
                None => eq,
            });
        }
        let matched = cond.unwrap_or_else(|| TExpr::new(TExprKind::Bool(false), Type::Bool, span));
        let not_matched = TExpr::new(TExprKind::Not(bx(matched)), Type::Bool, span);
        let msg = format!("`as` cast failed: the string is not one of {}", lits.join(", "));
        let fail = TStmt::Trap(TExpr::new(TExprKind::Str(msg), Type::String, span));
        let guard = TStmt::If(not_matched, vec![fail], Vec::new());
        let result = TExpr::new(TExprKind::Coerce(Coercion::Retag, bx(v_r)), Type::StrLits(lits.clone()), span);
        TExpr::new(TExprKind::Block(vec![TStmt::Let(v, Some(self.coerce(te, &Type::String))), guard], bx(result)), Type::StrLits(lits), span)
    }

    /// `union as Member`: trap if the union's discriminant tag does not
    /// match the target member's literal tag.
    fn cast_union_to_member(&mut self, te: TExpr, members: Rc<[Type]>, target: Type, span: Span) -> TExpr {
        let union_ty = Type::Union(members.clone());
        let Some((_, lits)) = self.union_discriminant(&members) else {
            self.err_help(
                code::TYPE_ASSERTION,
                span,
                "cannot cast: this union has no shared discriminant field to check at run time",
                "narrow it first with a discriminant field comparison",
            );
            return TExpr::new(TExprKind::Null, Type::Error, span);
        };
        let target_idx = members.iter().position(|m| *m == target).expect("checked by caller");
        let Some((lit, _)) = lits.iter().find(|(_, idx)| *idx == target_idx) else {
            self.err(code::TYPE_ASSERTION, span, "cannot cast: this union member has no discriminant literal");
            return TExpr::new(TExprKind::Null, Type::Error, span);
        };
        let lit = lit.clone();
        let all_lits = Type::str_lits(lits.iter().map(|(l, _)| l.clone()).collect());
        let v = self.temp(union_ty.clone());
        let v_r = TExpr::new(TExprKind::Var(v), union_ty.clone(), span);
        let tag = TExpr::new(TExprKind::UnionTag(bx(v_r.clone())), all_lits, span);
        let eq = TExpr::new(
            TExprKind::Cmp(CmpOp::Eq, EqKind::Str, bx(tag), bx(TExpr::new(TExprKind::Str(lit.clone()), Type::String, span))),
            Type::Bool,
            span,
        );
        let not_matched = TExpr::new(TExprKind::Not(bx(eq)), Type::Bool, span);
        let msg = format!("`as` cast failed: the value's tag is not \"{lit}\"");
        let fail = TStmt::Trap(TExpr::new(TExprKind::Str(msg), Type::String, span));
        let guard = TStmt::If(not_matched, vec![fail], Vec::new());
        let result = TExpr::new(TExprKind::Coerce(Coercion::Retag, bx(v_r)), target.clone(), span);
        TExpr::new(TExprKind::Block(vec![TStmt::Let(v, Some(te)), guard], bx(result)), target, span)
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
            Type::Struct(sid) => {
                if let Some(info) = self.promise_of(&o.ty).cloned() {
                    return self.promise_method(o, info, prop, prop_span, args, span);
                }
                if let Some(fid) = self.resolve_method(sid, prop) {
                    let ft = self.func_type(fid, prop_span);
                    let mut targs = self.method_args(&ft, o, args, span);
                    let this = targs.remove(0);
                    // Codegen resolves this to a direct call, or to an
                    // inline dispatch on the receiver's runtime type, once
                    // the whole program's class hierarchy is known
                    // (SPEC.md §4.2 v1 overriding).
                    return TExpr::new(TExprKind::MethodCall(sid, prop.to_string(), bx(this), targs), ft.ret.clone(), span);
                }
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
            "lastIndexOf" => rt("str_last_index_of", vec![o, self.str_arg(args, 0, span)], Type::Number),
            "replace" => {
                let a = self.str_arg(args, 0, span);
                let b = self.str_arg(args, 1, span);
                rt("str_replace", vec![o, a, b], Type::String)
            }
            "replaceAll" => {
                let a = self.str_arg(args, 0, span);
                let b = self.str_arg(args, 1, span);
                rt("str_replace_all", vec![o, a, b], Type::String)
            }
            "padStart" => {
                let n = self.num_arg(args, 0, 0.0, span);
                let pad = match args.get(1) {
                    Some(_) => self.str_arg(args, 1, span),
                    None => TExpr::new(TExprKind::Str(" ".into()), Type::String, span),
                };
                rt("str_pad_start", vec![o, n, pad], Type::String)
            }
            "padEnd" => {
                let n = self.num_arg(args, 0, 0.0, span);
                let pad = match args.get(1) {
                    Some(_) => self.str_arg(args, 1, span),
                    None => TExpr::new(TExprKind::Str(" ".into()), Type::String, span),
                };
                rt("str_pad_end", vec![o, n, pad], Type::String)
            }
            "charAt" => {
                let a = self.num_arg(args, 0, 0.0, span);
                let a2 = match &a.kind {
                    TExprKind::Num(v) => TExpr::new(TExprKind::Num(v + 1.0), Type::Number, span),
                    _ => {
                        let one = TExpr::new(TExprKind::Num(1.0), Type::Number, span);
                        TExpr::new(TExprKind::Num2(NumOp::Add, bx(a.clone()), bx(one)), Type::Number, span)
                    }
                };
                rt("str_slice", vec![o, a, a2], Type::String)
            }
            "split" => {
                let sep = self.str_arg(args, 0, span);
                rt("str_split", vec![o, sep], Type::Array(Box::new(Type::String)))
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
                match elem.repr() {
                    crate::types::Repr::F64 | crate::types::Repr::I32 => {
                        // `arr_pop_{f64,i32}` on an empty array returns a
                        // sentinel (`NaN` / `0`), not a signal the checker
                        // can box as null: boxing it unconditionally would
                        // give a non-null `NaN`/`0` instead of `null`
                        // (HANDOFF.md item 2 found this while adding boxing
                        // for `int`/`boolean`/enum elements). Check first.
                        let arr_v = self.temp(arr_ty.clone());
                        let arr_r = TExpr::new(TExprKind::Var(arr_v), arr_ty.clone(), span);
                        let is_empty = TExpr::new(
                            TExprKind::Cmp(
                                CmpOp::Eq,
                                EqKind::F64,
                                bx(self.arr_len_of(arr_r.clone(), span)),
                                bx(TExpr::new(TExprKind::Num(0.0), Type::Number, span)),
                            ),
                            Type::Bool,
                            span,
                        );
                        let null_v = self.coerce(TExpr::new(TExprKind::Null, Type::Null, span), &r);
                        let (rt_fn, box_c) = if elem.repr() == crate::types::Repr::F64 {
                            ("arr_pop_f64", Coercion::BoxNum)
                        } else {
                            ("arr_pop_i32", Coercion::BoxI32)
                        };
                        let v = TExpr::new(TExprKind::Rt(rt_fn, vec![arr_r]), elem.clone(), span);
                        let popped = TExpr::new(TExprKind::Coerce(box_c, bx(v)), r.clone(), span);
                        let cond = TExpr::new(TExprKind::Cond(bx(is_empty), bx(null_v), bx(popped)), r.clone(), span);
                        TExpr::new(TExprKind::Block(vec![TStmt::Let(arr_v, Some(o))], bx(cond)), r, span)
                    }
                    _ => {
                        // Already a `Ref`: `0` already means null, and
                        // `arr_pop_i32` on an empty array returns exactly
                        // that `0` without boxing it into something else.
                        TExpr::new(TExprKind::Rt("arr_pop_i32", vec![o]), r, span)
                    }
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
            // `a.concat(b, c)` is exactly `[...a, ...b, ...c]`: reuse the
            // array literal's existing spread handling (`arr_extend`,
            // HANDOFF.md item 1), so no new runtime function is needed.
            "concat" => {
                let mut items = vec![(true, o)];
                for a in args {
                    let te = self.expr_with(a, &arr_ty);
                    let te = self.coerce(te, &arr_ty);
                    items.push((true, te));
                }
                TExpr::new(TExprKind::ArrayLit(items), arr_ty, span)
            }
            // `arr.reduce((acc, item, index) => ..., initial)`: a plain
            // index loop built from existing TIR (`Let`/`Loop`/
            // `CallClosure`/`Assign`), the same approach `kv_for_of` uses,
            // so this needs no new runtime function or codegen support.
            // Only the two-argument form (with an explicit initial value)
            // is supported: without one, the accumulator's type would have
            // to be inferred from an array that might be empty, which
            // TypeScript itself only resolves by falling back to the
            // element type (and still traps at run time on an empty
            // array) — not worth the extra inference path here.
            "reduce" => {
                if args.len() != 2 {
                    self.err_help(
                        code::ARG_COUNT,
                        span,
                        "`reduce` takes a callback and an initial value",
                        "`arr.reduce((acc, item, index) => ..., initial)`; `reduce` without an initial value is not supported",
                    );
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
                let init = self.expr(&args[1], None);
                let acc_ty = self.widen(init.ty.clone());
                let init = self.coerce(init, &acc_ty);
                let (f, arity) = self.callback(&args[0], &[acc_ty.clone(), elem.clone(), Type::Number], Some(acc_ty.clone()));

                let arr_v = self.temp(arr_ty.clone());
                let arr_r = TExpr::new(TExprKind::Var(arr_v), arr_ty.clone(), span);
                let len = self.arr_len_of(arr_r.clone(), span);
                let len_v = self.temp(Type::Number);
                let len_r = TExpr::new(TExprKind::Var(len_v), Type::Number, span);
                let i_v = self.temp(Type::Number);
                let i_r = TExpr::new(TExprKind::Var(i_v), Type::Number, span);
                let acc_v = self.temp(acc_ty.clone());
                let acc_r = TExpr::new(TExprKind::Var(acc_v), acc_ty.clone(), span);
                let f_v = self.temp(f.ty.clone());
                let f_r = TExpr::new(TExprKind::Var(f_v), f.ty.clone(), span);

                let elem_at = self.arr_get_at(arr_r.clone(), i_r.clone(), elem.clone(), span);
                let mut call_args = vec![acc_r.clone(), elem_at, i_r.clone()];
                call_args.truncate(arity);
                let call = TExpr::new(TExprKind::CallClosure(bx(f_r), call_args), acc_ty.clone(), span);
                let update_acc = TStmt::Expr(TExpr::new(TExprKind::Assign(Place::Var(acc_v), bx(call)), acc_ty.clone(), span));
                let inc = TExpr::new(TExprKind::Num2(NumOp::Add, bx(i_r.clone()), bx(TExpr::new(TExprKind::Num(1.0), Type::Number, span))), Type::Number, span);
                let update = TExpr::new(TExprKind::Assign(Place::Var(i_v), bx(inc)), Type::Number, span);
                let cond = TExpr::new(TExprKind::Cmp(CmpOp::Lt, EqKind::F64, bx(i_r.clone()), bx(len_r)), Type::Bool, span);
                let id = self.prog.new_loop();
                let loop_stmt = TStmt::Loop { id, cond: Some(cond), test_after: false, update: Some(update), body: vec![update_acc] };
                let prelude = vec![
                    TStmt::Let(arr_v, Some(o)),
                    TStmt::Let(f_v, Some(f)),
                    TStmt::Let(len_v, Some(len)),
                    TStmt::Let(acc_v, Some(init)),
                    TStmt::Let(i_v, Some(TExpr::new(TExprKind::Num(0.0), Type::Number, span))),
                    loop_stmt,
                ];
                TExpr::new(TExprKind::Block(prelude, bx(acc_r)), acc_ty, span)
            }
            // `arr.sort(compare?)`: a generated stable insertion sort (a
            // plain index loop, like `reduce`/`kv_delete`'s shift loop), not
            // a new runtime function (GAPS.md "Larger items"). Insertion
            // sort is stable (equal keys never cross) and small to generate;
            // `O(n^2)` is accepted here since Plinth arrays are form-sized
            // state, not bulk data (SPEC.md §4.2).
            "sort" => self.array_sort(o, elem, args, span),
            // `splice`/`fill`/`flat`: generated code over the existing array
            // ops (`arr_slice`, `arr_extend`, push/pop, index loops), like
            // `reduce` and `sort`. No new runtime function.
            "splice" => self.array_splice(o, elem, args, span),
            "fill" => self.array_fill(o, elem, args, span),
            "flat" => self.array_flat(o, elem, args, span),
            _ => {
                self.err(code::NO_PROPERTY, prop_span, format!("arrays have no method `{prop}` in Plinth TS"));
                TExpr::new(TExprKind::Null, Type::Error, span)
            }
        }
    }

    /// `t[i]` on a tuple: `i` must be a number literal in range, so the
    /// read is a plain field read with a known type. An index past the
    /// fixed elements of a tuple with a rest element reads the rest array.
    fn tuple_index(&mut self, sid: crate::types::StructId, index: &Expr) -> Option<TupleAt> {
        let shape = self.tuple_shape(sid).expect("checked by the caller");
        let n = shape.fixed.len();
        let lit = match index.kind {
            ExprKind::Num(k) if k >= 0.0 && k.fract() == 0.0 && k < 1e9 => Some(k as usize),
            _ => None,
        };
        match (lit, &shape.rest) {
            (Some(k), _) if k < n => Some(TupleAt::Field(k as u32, shape.fixed[k].clone())),
            (Some(k), Some(rest)) => Some(TupleAt::Rest(shape.rest_field(), (k - n) as u32, rest.clone())),
            (_, rest) => {
                let name = &self.prog.structs[sid as usize].name;
                let msg = match rest {
                    None => format!("an index into `{name}` must be a number literal from 0 to {}", n.saturating_sub(1)),
                    Some(_) => format!("an index into `{name}` must be a number literal (0 or more)"),
                };
                self.err_help(code::COMPUTED_ACCESS, index.span, msg, "write `t[0]`, or destructure: `const [a, b] = t`");
                None
            }
        }
    }

    /// `t.length` of a tuple: the number of required elements, plus the
    /// optional elements up to the last one that is not null, plus the
    /// length of the rest array.
    fn tuple_length(&mut self, o: TExpr, sid: crate::types::StructId, span: Span) -> TExpr {
        let shape = self.tuple_shape(sid).expect("checked by the caller");
        if shape.required == shape.fixed.len() && shape.rest.is_none() {
            let n = self.num_lit(shape.fixed.len() as f64, span);
            return TExpr::new(TExprKind::Block(vec![TStmt::Expr(o)], bx(n)), Type::Number, span);
        }
        let tv = self.temp(o.ty.clone());
        let t = TExpr::new(TExprKind::Var(tv), o.ty.clone(), span);
        let nv = self.temp(Type::Number);
        let n = TExpr::new(TExprKind::Var(nv), Type::Number, span);
        let mut stmts = vec![TStmt::Let(tv, Some(o)), TStmt::Let(nv, Some(self.num_lit(shape.required as f64, span)))];
        for k in shape.required..shape.fixed.len() {
            let f = TExpr::new(TExprKind::Field(bx(t.clone()), sid, k as u32), shape.fixed[k].clone(), span);
            let present = TExpr::new(TExprKind::Not(bx(TExpr::new(TExprKind::IsNull(bx(f)), Type::Bool, span))), Type::Bool, span);
            let set = TStmt::Expr(TExpr::new(TExprKind::Assign(Place::Var(nv), bx(self.num_lit(k as f64 + 1.0, span))), Type::Number, span));
            stmts.push(TStmt::If(present, vec![set], Vec::new()));
        }
        let total = match &shape.rest {
            Some(r) => {
                let arr = TExpr::new(TExprKind::Field(bx(t), sid, shape.rest_field()), Type::Array(Box::new(r.clone())), span);
                TExpr::new(TExprKind::Num2(NumOp::Add, bx(n), bx(self.arr_len_of(arr, span))), Type::Number, span)
            }
            None => n,
        };
        TExpr::new(TExprKind::Block(stmts, bx(total)), Type::Number, span)
    }

    fn num_lit(&self, v: f64, span: Span) -> TExpr {
        TExpr::new(TExprKind::Num(v), Type::Number, span)
    }

    /// A `number` as the `i32` an array runtime function takes (saturating
    /// truncation; `NaN` becomes 0).
    fn num_to_i32(&self, e: TExpr, span: Span) -> TExpr {
        TExpr::new(TExprKind::Coerce(Coercion::NumToI32, bx(e)), Type::Bool, span)
    }

    /// `x` truncated toward zero (JS `ToIntegerOrInfinity`, saturated to the
    /// `i32` range, which is wider than any array).
    fn num_trunc(&self, e: TExpr, span: Span) -> TExpr {
        let i = self.num_to_i32(e, span);
        TExpr::new(TExprKind::Coerce(Coercion::I32ToNum, bx(i)), Type::Number, span)
    }

    /// The JS relative index rule (`slice`, `fill`): `x` truncated; a
    /// negative value counts back from `len`; the result is clamped to
    /// `[0, len]`. `len` must be a plain variable read.
    fn rel_index(&mut self, x: TExpr, len: TExpr, span: Span) -> TExpr {
        let x_v = self.temp(Type::Number);
        let x_r = TExpr::new(TExprKind::Var(x_v), Type::Number, span);
        let lt0 = |a: TExpr| TExpr::new(TExprKind::Cmp(CmpOp::Lt, EqKind::F64, bx(a), bx(TExpr::new(TExprKind::Num(0.0), Type::Number, span))), Type::Bool, span);
        let from_end = TExpr::new(TExprKind::Num2(NumOp::Add, bx(len.clone()), bx(x_r.clone())), Type::Number, span);
        let neg = TExpr::new(TExprKind::Cond(bx(lt0(from_end.clone())), bx(self.num_lit(0.0, span)), bx(from_end)), Type::Number, span);
        let gt_len = TExpr::new(TExprKind::Cmp(CmpOp::Gt, EqKind::F64, bx(x_r.clone()), bx(len.clone())), Type::Bool, span);
        let pos = TExpr::new(TExprKind::Cond(bx(gt_len), bx(len), bx(x_r.clone())), Type::Number, span);
        let cond = TExpr::new(TExprKind::Cond(bx(lt0(x_r)), bx(neg), bx(pos)), Type::Number, span);
        let init = self.num_trunc(x, span);
        TExpr::new(TExprKind::Block(vec![TStmt::Let(x_v, Some(init))], bx(cond)), Type::Number, span)
    }

    /// `arr.splice(start, deleteCount?, ...items)`: removes `deleteCount`
    /// elements at `start` (JS relative index), inserts `items` there, and
    /// returns the removed elements. Generated as: `tail = arr.slice(start)`;
    /// `removed = tail.slice(0, d)`; `rest = tail.slice(d)`; pop `arr` down
    /// to `start`; push `items`; append `rest`.
    fn array_splice(&mut self, o: TExpr, elem: Type, args: &[Expr], span: Span) -> TExpr {
        if args.is_empty() {
            self.err_help(code::ARG_COUNT, span, "`splice` needs a start index", "`arr.splice(start, deleteCount?, ...items)`");
            return TExpr::new(TExprKind::Null, Type::Error, span);
        }
        let arr_ty = Type::Array(Box::new(elem.clone()));
        let var = |v: VarId, ty: &Type| TExpr::new(TExprKind::Var(v), ty.clone(), span);
        let mut stmts = Vec::new();

        // Evaluate the receiver and every argument once, in source order.
        let arr_v = self.temp(arr_ty.clone());
        stmts.push(TStmt::Let(arr_v, Some(o)));
        let start = self.num_arg(args, 0, 0.0, span);
        let start_v = self.temp(Type::Number);
        stmts.push(TStmt::Let(start_v, Some(start)));
        let dc_v = if args.len() >= 2 {
            let dc = self.num_arg(args, 1, 0.0, span);
            let dc_v = self.temp(Type::Number);
            stmts.push(TStmt::Let(dc_v, Some(dc)));
            Some(dc_v)
        } else {
            None
        };
        let mut items = Vec::new();
        for a in args.iter().skip(2) {
            let te = self.expr_with(a, &elem);
            let te = self.coerce(te, &elem);
            let v = self.temp(elem.clone());
            stmts.push(TStmt::Let(v, Some(te)));
            items.push(v);
        }

        let arr_r = var(arr_v, &arr_ty);
        let max = || TExpr::new(TExprKind::Num(i32::MAX as f64), Type::Number, span);
        let tail_v = self.temp(arr_ty.clone());
        let tail_r = var(tail_v, &arr_ty);
        let tail = TExpr::new(
            TExprKind::Rt("arr_slice", vec![arr_r.clone(), self.num_to_i32(var(start_v, &Type::Number), span), self.num_to_i32(max(), span)]),
            arr_ty.clone(),
            span,
        );
        stmts.push(TStmt::Let(tail_v, Some(tail)));
        // `s = arr.length - tail.length` is the resolved start index.
        let s_v = self.temp(Type::Number);
        let s_init = TExpr::new(
            TExprKind::Num2(NumOp::Sub, bx(self.arr_len_of(arr_r.clone(), span)), bx(self.arr_len_of(tail_r.clone(), span))),
            Type::Number,
            span,
        );
        stmts.push(TStmt::Let(s_v, Some(s_init)));
        // `d`: the delete count, truncated, at least 0 (`slice` clamps the
        // top). No count means "to the end".
        let d_v = self.temp(Type::Number);
        let d_init = match dc_v {
            Some(dc_v) => {
                let t = self.num_trunc(var(dc_v, &Type::Number), span);
                let t_v = self.temp(Type::Number);
                let t_r = var(t_v, &Type::Number);
                let lt0 = TExpr::new(TExprKind::Cmp(CmpOp::Lt, EqKind::F64, bx(t_r.clone()), bx(self.num_lit(0.0, span))), Type::Bool, span);
                let c = TExpr::new(TExprKind::Cond(bx(lt0), bx(self.num_lit(0.0, span)), bx(t_r)), Type::Number, span);
                TExpr::new(TExprKind::Block(vec![TStmt::Let(t_v, Some(t))], bx(c)), Type::Number, span)
            }
            None => self.arr_len_of(tail_r.clone(), span),
        };
        stmts.push(TStmt::Let(d_v, Some(d_init)));
        let d_i32 = || TExpr::new(TExprKind::Coerce(Coercion::NumToI32, bx(TExpr::new(TExprKind::Var(d_v), Type::Number, span))), Type::Bool, span);
        let removed_v = self.temp(arr_ty.clone());
        let removed = TExpr::new(TExprKind::Rt("arr_slice", vec![tail_r.clone(), self.num_to_i32(self.num_lit(0.0, span), span), d_i32()]), arr_ty.clone(), span);
        stmts.push(TStmt::Let(removed_v, Some(removed)));
        let rest_v = self.temp(arr_ty.clone());
        let rest = TExpr::new(TExprKind::Rt("arr_slice", vec![tail_r, d_i32(), self.num_to_i32(max(), span)]), arr_ty.clone(), span);
        stmts.push(TStmt::Let(rest_v, Some(rest)));

        // `while (arr.length > s) arr.pop();`
        let cond = TExpr::new(
            TExprKind::Cmp(CmpOp::Gt, EqKind::F64, bx(self.arr_len_of(arr_r.clone(), span)), bx(var(s_v, &Type::Number))),
            Type::Bool,
            span,
        );
        let id = self.prog.new_loop();
        let pop = self.arr_pop_discard(arr_r.clone(), span);
        stmts.push(TStmt::Loop { id, cond: Some(cond), test_after: false, update: None, body: vec![pop] });
        for v in items {
            stmts.push(self.arr_push_discard(arr_r.clone(), var(v, &elem), span));
        }
        stmts.push(TStmt::Expr(TExpr::new(TExprKind::Rt("arr_extend", vec![arr_r, var(rest_v, &arr_ty)]), Type::Void, span)));
        TExpr::new(TExprKind::Block(stmts, bx(var(removed_v, &arr_ty))), arr_ty, span)
    }

    /// `arr.fill(value, start?, end?)`: in place, returns the same array.
    /// `start` and `end` use the JS relative index rule.
    fn array_fill(&mut self, o: TExpr, elem: Type, args: &[Expr], span: Span) -> TExpr {
        if args.is_empty() || args.len() > 3 {
            self.err_help(code::ARG_COUNT, span, "`fill` takes a value and an optional start and end", "`arr.fill(value, start?, end?)`");
            return TExpr::new(TExprKind::Null, Type::Error, span);
        }
        let arr_ty = Type::Array(Box::new(elem.clone()));
        let var = |v: VarId, ty: &Type| TExpr::new(TExprKind::Var(v), ty.clone(), span);
        let mut stmts = Vec::new();
        let arr_v = self.temp(arr_ty.clone());
        stmts.push(TStmt::Let(arr_v, Some(o)));
        let arr_r = var(arr_v, &arr_ty);
        let value = self.expr_with(&args[0], &elem);
        let value = self.coerce(value, &elem);
        let value_v = self.temp(elem.clone());
        stmts.push(TStmt::Let(value_v, Some(value)));
        let len_v = self.temp(Type::Number);
        stmts.push(TStmt::Let(len_v, Some(self.arr_len_of(arr_r.clone(), span))));
        let len_r = var(len_v, &Type::Number);
        let i_v = self.temp(Type::Number);
        let i_r = var(i_v, &Type::Number);
        let start = self.num_arg(args, 1, 0.0, span);
        let start = self.rel_index(start, len_r.clone(), span);
        stmts.push(TStmt::Let(i_v, Some(start)));
        let end_v = self.temp(Type::Number);
        let end = if args.len() >= 3 {
            let e = self.num_arg(args, 2, 0.0, span);
            self.rel_index(e, len_r.clone(), span)
        } else {
            len_r
        };
        stmts.push(TStmt::Let(end_v, Some(end)));

        let set = self.arr_set_at(arr_r.clone(), i_r.clone(), var(value_v, &elem), span);
        let cond = TExpr::new(TExprKind::Cmp(CmpOp::Lt, EqKind::F64, bx(i_r.clone()), bx(var(end_v, &Type::Number))), Type::Bool, span);
        let inc = TExpr::new(TExprKind::Num2(NumOp::Add, bx(i_r), bx(self.num_lit(1.0, span))), Type::Number, span);
        let update = TExpr::new(TExprKind::Assign(Place::Var(i_v), bx(inc)), Type::Number, span);
        let id = self.prog.new_loop();
        stmts.push(TStmt::Loop { id, cond: Some(cond), test_after: false, update: Some(update), body: vec![set] });
        TExpr::new(TExprKind::Block(stmts, bx(arr_r)), arr_ty, span)
    }

    /// `arr.flat()`: one level. On a `T[][]` it appends every inner array to
    /// a new `T[]`; on any other array it returns a copy (as JS does). A
    /// depth other than a literal `1` is not supported.
    /// `arr.flat(depth?)`. The result type depends on the depth, so the
    /// depth must be a number literal or `Infinity`. Each level is one
    /// generated loop (`flat_once`); a depth larger than the nesting of
    /// the array flattens all levels, as in JS.
    fn array_flat(&mut self, o: TExpr, elem: Type, args: &[Expr], span: Span) -> TExpr {
        if args.len() > 1 {
            self.err(code::ARG_COUNT, span, "`flat` takes at most one depth");
            return TExpr::new(TExprKind::Null, Type::Error, span);
        }
        let depth = match args.first().map(|a| &a.kind) {
            None => 1.0,
            Some(ExprKind::Num(n)) => n.trunc().max(0.0),
            Some(ExprKind::Ident(name)) if name == "Infinity" && self.lookup(name).is_none() => f64::INFINITY,
            Some(_) => {
                let a = &args[0];
                self.err_help(
                    code::UNSUPPORTED,
                    a.span,
                    "the depth of `flat` must be a number literal or `Infinity`",
                    "write the depth as a number, such as `flat(2)`",
                );
                return TExpr::new(TExprKind::Null, Type::Error, span);
            }
        };
        if depth == 0.0 {
            return self.arr_copy_of(o, span);
        }
        let mut out = self.flat_once(o, elem, span);
        let mut done = 1.0;
        while done < depth {
            let Type::Array(e) = out.ty.clone() else { unreachable!() };
            if !matches!(*e, Type::Array(_)) {
                break;
            }
            out = self.flat_once(out, *e, span);
            done += 1.0;
        }
        out
    }

    /// One level of `flat`: a `T[][]` becomes a `T[]`; any other array is
    /// copied.
    fn flat_once(&mut self, o: TExpr, elem: Type, span: Span) -> TExpr {
        let inner = match &elem {
            Type::Array(inner) => (**inner).clone(),
            _ => return self.arr_copy_of(o, span),
        };
        let arr_ty = Type::Array(Box::new(elem.clone()));
        let res_ty = Type::Array(Box::new(inner));
        let var = |v: VarId, ty: &Type| TExpr::new(TExprKind::Var(v), ty.clone(), span);
        let arr_v = self.temp(arr_ty.clone());
        let arr_r = var(arr_v, &arr_ty);
        let res_v = self.temp(res_ty.clone());
        let res_r = var(res_v, &res_ty);
        let len_v = self.temp(Type::Number);
        let i_v = self.temp(Type::Number);
        let i_r = var(i_v, &Type::Number);
        let sub = self.arr_get_at(arr_r.clone(), i_r.clone(), elem.clone(), span);
        let extend = TStmt::Expr(TExpr::new(TExprKind::Rt("arr_extend", vec![res_r.clone(), sub]), Type::Void, span));
        let cond = TExpr::new(TExprKind::Cmp(CmpOp::Lt, EqKind::F64, bx(i_r.clone()), bx(var(len_v, &Type::Number))), Type::Bool, span);
        let inc = TExpr::new(TExprKind::Num2(NumOp::Add, bx(i_r), bx(self.num_lit(1.0, span))), Type::Number, span);
        let update = TExpr::new(TExprKind::Assign(Place::Var(i_v), bx(inc)), Type::Number, span);
        let id = self.prog.new_loop();
        let stmts = vec![
            TStmt::Let(arr_v, Some(o)),
            TStmt::Let(res_v, Some(TExpr::new(TExprKind::ArrayLit(Vec::new()), res_ty.clone(), span))),
            TStmt::Let(len_v, Some(self.arr_len_of(arr_r, span))),
            TStmt::Let(i_v, Some(self.num_lit(0.0, span))),
            TStmt::Loop { id, cond: Some(cond), test_after: false, update: Some(update), body: vec![extend] },
        ];
        TExpr::new(TExprKind::Block(stmts, bx(res_r)), res_ty, span)
    }

    /// `arr.sort(compare?)`. In place, returns the same array (JS
    /// semantics). Without a comparator, JS compares the elements' string
    /// forms; only `string`/`number`/`boolean` elements support that
    /// fallback (a `number` fallback also gets a lint, since comparing
    /// numbers as strings is almost never intended), anything else needs an
    /// explicit comparator.
    fn array_sort(&mut self, o: TExpr, elem: Type, args: &[Expr], span: Span) -> TExpr {
        if args.len() > 1 {
            self.err(code::ARG_COUNT, span, "`sort` takes at most one comparator");
            return TExpr::new(TExprKind::Null, Type::Error, span);
        }
        let arr_ty = Type::Array(Box::new(elem.clone()));

        enum Cmp {
            Closure(VarId, usize, Type),
            Str,
            NumAsStr,
            BoolAsStr,
        }

        let mut prelude: Vec<TStmt> = Vec::new();
        let cmp = if let Some(cmp_arg) = args.first() {
            let (f, arity) = self.callback(cmp_arg, &[elem.clone(), elem.clone()], Some(Type::Number));
            let f_ty = f.ty.clone();
            let f_v = self.temp(f_ty.clone());
            prelude.push(TStmt::Let(f_v, Some(f)));
            Cmp::Closure(f_v, arity, f_ty)
        } else {
            match self.widen(elem.clone()) {
                Type::String => Cmp::Str,
                Type::Number => {
                    self.diags.push(crate::diag::Diagnostic::warning(
                        code::SORT_DEFAULT_COMPARE,
                        span,
                        "sort() without a comparator compares numbers as strings; pass (a, b) => a - b",
                    ));
                    Cmp::NumAsStr
                }
                Type::Bool => Cmp::BoolAsStr,
                _ => {
                    self.err_help(
                        code::ARG_COUNT,
                        span,
                        "`sort` needs a comparator for this element type",
                        "pass `(a, b) => ...` returning a negative, zero or positive number",
                    );
                    return TExpr::new(TExprKind::Null, Type::Error, span);
                }
            }
        };

        // `cmp_gt(a, b)`: true when `a` must sort after `b`.
        let cmp_gt = |a: TExpr, b: TExpr| -> TExpr {
            match &cmp {
                Cmp::Closure(f_v, arity, f_ty) => {
                    let f_r = TExpr::new(TExprKind::Var(*f_v), f_ty.clone(), span);
                    let mut call_args = vec![a, b];
                    call_args.truncate(*arity);
                    let call = TExpr::new(TExprKind::CallClosure(bx(f_r), call_args), Type::Number, span);
                    TExpr::new(
                        TExprKind::Cmp(CmpOp::Gt, EqKind::F64, bx(call), bx(TExpr::new(TExprKind::Num(0.0), Type::Number, span))),
                        Type::Bool,
                        span,
                    )
                }
                Cmp::Str => TExpr::new(TExprKind::StrCmp(CmpOp::Gt, bx(a), bx(b)), Type::Bool, span),
                Cmp::NumAsStr => {
                    let a = TExpr::new(TExprKind::Coerce(Coercion::NumToStr, bx(a)), Type::String, span);
                    let b = TExpr::new(TExprKind::Coerce(Coercion::NumToStr, bx(b)), Type::String, span);
                    TExpr::new(TExprKind::StrCmp(CmpOp::Gt, bx(a), bx(b)), Type::Bool, span)
                }
                Cmp::BoolAsStr => {
                    let a = TExpr::new(TExprKind::Coerce(Coercion::BoolToStr, bx(a)), Type::String, span);
                    let b = TExpr::new(TExprKind::Coerce(Coercion::BoolToStr, bx(b)), Type::String, span);
                    TExpr::new(TExprKind::StrCmp(CmpOp::Gt, bx(a), bx(b)), Type::Bool, span)
                }
            }
        };

        let arr_v = self.temp(arr_ty.clone());
        let arr_r = TExpr::new(TExprKind::Var(arr_v), arr_ty.clone(), span);
        let n_v = self.temp(Type::Number);
        let n_r = TExpr::new(TExprKind::Var(n_v), Type::Number, span);
        let i_v = self.temp(Type::Number);
        let i_r = TExpr::new(TExprKind::Var(i_v), Type::Number, span);
        let j_v = self.temp(Type::Number);
        let j_r = TExpr::new(TExprKind::Var(j_v), Type::Number, span);
        let key_v = self.temp(elem.clone());
        let key_r = TExpr::new(TExprKind::Var(key_v), elem.clone(), span);

        let one = || TExpr::new(TExprKind::Num(1.0), Type::Number, span);
        let zero = || TExpr::new(TExprKind::Num(0.0), Type::Number, span);

        // `key = arr[i]; j = i - 1;`
        let assign_key =
            TStmt::Expr(TExpr::new(TExprKind::Assign(Place::Var(key_v), bx(self.arr_get_at(arr_r.clone(), i_r.clone(), elem.clone(), span))), elem.clone(), span));
        let j_init = TExpr::new(TExprKind::Num2(NumOp::Sub, bx(i_r.clone()), bx(one())), Type::Number, span);
        let assign_j = TStmt::Expr(TExpr::new(TExprKind::Assign(Place::Var(j_v), bx(j_init)), Type::Number, span));

        // Inner loop: `while (j >= 0 && cmp_gt(arr[j], key)) { arr[j+1] = arr[j]; j--; }`
        let j_plus_1 = TExpr::new(TExprKind::Num2(NumOp::Add, bx(j_r.clone()), bx(one())), Type::Number, span);
        let ge0 = TExpr::new(TExprKind::Cmp(CmpOp::Ge, EqKind::F64, bx(j_r.clone()), bx(zero())), Type::Bool, span);
        let shift_needed = cmp_gt(self.arr_get_at(arr_r.clone(), j_r.clone(), elem.clone(), span), key_r.clone());
        let inner_cond =
            TExpr::new(TExprKind::Cond(bx(ge0), bx(shift_needed), bx(TExpr::new(TExprKind::Bool(false), Type::Bool, span))), Type::Bool, span);
        let shift = self.arr_set_at(arr_r.clone(), j_plus_1.clone(), self.arr_get_at(arr_r.clone(), j_r.clone(), elem.clone(), span), span);
        let j_dec = TExpr::new(TExprKind::Num2(NumOp::Sub, bx(j_r.clone()), bx(one())), Type::Number, span);
        let assign_j_dec = TStmt::Expr(TExpr::new(TExprKind::Assign(Place::Var(j_v), bx(j_dec)), Type::Number, span));
        let inner_id = self.prog.new_loop();
        let inner_loop = TStmt::Loop { id: inner_id, cond: Some(inner_cond), test_after: false, update: None, body: vec![shift, assign_j_dec] };

        // `arr[j+1] = key;`
        let place_final = self.arr_set_at(arr_r.clone(), j_plus_1, key_r.clone(), span);

        let outer_cond = TExpr::new(TExprKind::Cmp(CmpOp::Lt, EqKind::F64, bx(i_r.clone()), bx(n_r.clone())), Type::Bool, span);
        let i_inc = TExpr::new(TExprKind::Num2(NumOp::Add, bx(i_r.clone()), bx(one())), Type::Number, span);
        let update = TExpr::new(TExprKind::Assign(Place::Var(i_v), bx(i_inc)), Type::Number, span);
        let outer_id = self.prog.new_loop();
        let outer_loop = TStmt::Loop {
            id: outer_id,
            cond: Some(outer_cond),
            test_after: false,
            update: Some(update),
            body: vec![assign_key, assign_j, inner_loop, place_final],
        };

        prelude.push(TStmt::Let(arr_v, Some(o)));
        prelude.push(TStmt::Let(n_v, Some(self.arr_len_of(arr_r.clone(), span))));
        prelude.push(TStmt::Let(key_v, None));
        prelude.push(TStmt::Let(j_v, None));
        prelude.push(TStmt::Let(i_v, Some(one())));
        prelude.push(outer_loop);

        TExpr::new(TExprKind::Block(prelude, bx(arr_r)), arr_ty, span)
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

    /// A fresh copy of `arr` (`arr_slice(arr, 0, MAX)`, the same op `.slice()`
    /// uses): `Map`/`Set`'s `.keys()`/`.values()` return this, not the raw
    /// field, so pushing/popping the result cannot corrupt the map/set's own
    /// backing arrays.
    pub(super) fn arr_copy_of(&self, arr: TExpr, span: Span) -> TExpr {
        let ty = arr.ty.clone();
        let zero = TExpr::new(TExprKind::Coerce(Coercion::NumToI32, bx(TExpr::new(TExprKind::Num(0.0), Type::Number, span))), Type::Bool, span);
        let max = TExpr::new(
            TExprKind::Coerce(Coercion::NumToI32, bx(TExpr::new(TExprKind::Num(i32::MAX as f64), Type::Number, span))),
            Type::Bool,
            span,
        );
        TExpr::new(TExprKind::Rt("arr_slice", vec![arr, zero, max]), ty, span)
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

    pub(super) fn arr_push_discard(&self, arr: TExpr, value: TExpr, span: Span) -> TStmt {
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

    /// `m.delete(k)` / `s.delete(v)`: finds the key, then shifts every later
    /// entry down by one and pops, so `delete` keeps insertion order
    /// (HANDOFF.md item 1). `for (let j = idx; j < last; j = j + 1) { keys[j]
    /// = keys[j+1]; values[j] = values[j+1]; }` then one pop per array.
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
        let keys_elem = match &keys.ty {
            Type::Array(e) => (**e).clone(),
            _ => unreachable!(),
        };

        let j_v = self.temp(Type::Number);
        let j_r = TExpr::new(TExprKind::Var(j_v), Type::Number, span);
        let j_plus_1 =
            TExpr::new(TExprKind::Num2(NumOp::Add, bx(j_r.clone()), bx(TExpr::new(TExprKind::Num(1.0), Type::Number, span))), Type::Number, span);
        let shift_cond = TExpr::new(TExprKind::Cmp(CmpOp::Lt, EqKind::F64, bx(j_r.clone()), bx(last_r.clone())), Type::Bool, span);
        let mut shift_body = vec![self.arr_set_at(keys.clone(), j_r.clone(), self.arr_get_at(keys.clone(), j_plus_1.clone(), keys_elem, span), span)];
        if has_values {
            let values = self.kv_field(&obj_r, sid, 1, span);
            let values_elem = match &values.ty {
                Type::Array(e) => (**e).clone(),
                _ => unreachable!(),
            };
            shift_body.push(self.arr_set_at(values.clone(), j_r.clone(), self.arr_get_at(values, j_plus_1.clone(), values_elem, span), span));
        }
        let shift_update = TExpr::new(TExprKind::Assign(Place::Var(j_v), bx(j_plus_1)), Type::Number, span);
        let shift_id = self.prog.new_loop();
        let shift_loop = TStmt::Loop { id: shift_id, cond: Some(shift_cond), test_after: false, update: Some(shift_update), body: shift_body };

        let mut pop_stmts = vec![TStmt::Let(j_v, Some(idx_r.clone())), shift_loop, self.arr_pop_discard(keys, span)];
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

    /// `for…of` over a `Map` or `Set` (HANDOFF.md item 1): a bare `for (x of
    /// set)`/`for ([k, v] of map)`, or `.keys()`/`.values()`/`.entries()`.
    /// `None` means `iter` is not one of these forms, so the caller falls
    /// back to the array `for…of`. Lowers to an index loop over the
    /// `keys`/`values` arrays (see the comment above `kv_delete`): no new
    /// runtime or codegen support is needed.
    pub(super) fn kv_for_of(&mut self, kind: ast::VarKind, pattern: &ast::Pattern, iter: &Expr, body: &ast::Stmt) -> Option<Vec<TStmt>> {
        #[derive(Clone, Copy, PartialEq)]
        enum Mode {
            Keys,
            Values,
            Entries,
        }

        let span = iter.span;
        let (obj, mode) = if let ExprKind::Call { callee, args, .. } = &iter.kind {
            if !args.is_empty() {
                return None;
            }
            let ExprKind::Member { obj, prop, .. } = &callee.kind else { return None };
            let mode = match prop.as_str() {
                "keys" => Mode::Keys,
                "values" => Mode::Values,
                "entries" => Mode::Entries,
                _ => return None,
            };
            let o = self.expr(obj, None);
            if !matches!(o.ty, Type::Map(..) | Type::Set(_)) {
                return None;
            }
            (o, mode)
        } else {
            let o = self.expr(iter, None);
            match &o.ty {
                Type::Map(..) => (o, Mode::Entries),
                Type::Set(_) => (o, Mode::Keys),
                _ => return None,
            }
        };

        let (sid, k_ty, v_ty) = match &obj.ty {
            Type::Map(k, v) => (self.map_struct(k, v), (**k).clone(), Some((**v).clone())),
            Type::Set(t) => (self.set_struct(t), (**t).clone(), None),
            _ => unreachable!(),
        };
        let mode = if matches!(obj.ty, Type::Set(_)) && mode != Mode::Keys {
            let what = if mode == Mode::Values { "values()" } else { "entries()" };
            self.err(code::NO_PROPERTY, span, format!("`Set` has no `{what}`; use a bare `for…of` or `.keys()`"));
            Mode::Keys
        } else {
            mode
        };

        let mutable = kind == ast::VarKind::Let;
        let id = self.prog.new_loop();

        let obj_v = self.temp(obj.ty.clone());
        let obj_r = TExpr::new(TExprKind::Var(obj_v), obj.ty.clone(), span);
        let keys = self.kv_field(&obj_r, sid, 0, span);
        let len = self.arr_len_of(keys.clone(), span);
        let len_v = self.temp(Type::Number);
        let len_r = TExpr::new(TExprKind::Var(len_v), Type::Number, span);
        let i_v = self.temp(Type::Number);
        let i_r = TExpr::new(TExprKind::Var(i_v), Type::Number, span);

        self.fx.loops.push(id);
        self.push_scope();

        let mut prologue = Vec::new();
        match (mode, pattern) {
            (Mode::Keys, ast::Pattern::Ident(name, pspan)) => {
                let v = self.new_var(name, k_ty.clone(), mutable);
                self.define(name, *pspan, Binding::Var(v));
                let get = self.arr_get_at(keys.clone(), i_r.clone(), k_ty.clone(), span);
                prologue.push(TStmt::Let(v, Some(get)));
            }
            (Mode::Values, ast::Pattern::Ident(name, pspan)) => {
                let vt = v_ty.clone().unwrap_or(Type::Error);
                let values = self.kv_field(&obj_r, sid, 1, span);
                let v = self.new_var(name, vt.clone(), mutable);
                self.define(name, *pspan, Binding::Var(v));
                let get = self.arr_get_at(values, i_r.clone(), vt, span);
                prologue.push(TStmt::Let(v, Some(get)));
            }
            (Mode::Entries, ast::Pattern::Array(elems, None, pspan)) if elems.len() == 2 => match (&elems[0], &elems[1]) {
                (Some(ast::Pattern::Ident(kn, kspan)), Some(ast::Pattern::Ident(vn, vspan))) => {
                    let vt = v_ty.clone().unwrap_or(Type::Error);
                    let kv = self.new_var(kn, k_ty.clone(), mutable);
                    self.define(kn, *kspan, Binding::Var(kv));
                    let kget = self.arr_get_at(keys.clone(), i_r.clone(), k_ty.clone(), span);
                    prologue.push(TStmt::Let(kv, Some(kget)));
                    let values = self.kv_field(&obj_r, sid, 1, span);
                    let vv = self.new_var(vn, vt.clone(), mutable);
                    self.define(vn, *vspan, Binding::Var(vv));
                    let vget = self.arr_get_at(values, i_r.clone(), vt, span);
                    prologue.push(TStmt::Let(vv, Some(vget)));
                }
                _ => {
                    self.err(code::TYPE_MISMATCH, *pspan, "`for…of` over a `Map` needs `[key, value]` with simple names");
                }
            },
            (Mode::Entries, ast::Pattern::Ident(name, pspan)) => {
                // `for (const e of m)`: `e` is a `[K, V]` pair.
                let vt = v_ty.clone().unwrap_or(Type::Error);
                let pair = self.tuple_struct(&[k_ty.clone(), vt.clone()]);
                let values = self.kv_field(&obj_r, sid, 1, span);
                let lit = TExpr::new(
                    TExprKind::StructLit(pair, vec![self.arr_get_at(keys.clone(), i_r.clone(), k_ty.clone(), span), self.arr_get_at(values, i_r.clone(), vt, span)]),
                    Type::Struct(pair),
                    span,
                );
                let v = self.new_var(name, Type::Struct(pair), mutable);
                self.define(name, *pspan, Binding::Var(v));
                prologue.push(TStmt::Let(v, Some(lit)));
            }
            (Mode::Entries, other) => {
                self.err(code::TYPE_MISMATCH, other.span(), "`for…of` over a `Map` needs `[key, value]`, for example `for (const [k, v] of m)`");
            }
            (_, other) => {
                self.err(code::TYPE_MISMATCH, other.span(), "`for…of` over a `Set` needs a single name, for example `for (const v of s)`");
            }
        }

        let b = match &body.kind {
            ast::StmtKind::Block(b) => self.block_stmts(b),
            _ => self.stmt(body, false),
        };
        prologue.extend(b);
        self.pop_scope();
        self.fx.loops.pop();

        let cond = TExpr::new(TExprKind::Cmp(CmpOp::Lt, EqKind::F64, bx(i_r.clone()), bx(len_r)), Type::Bool, span);
        let inc = TExpr::new(TExprKind::Num2(NumOp::Add, bx(i_r), bx(TExpr::new(TExprKind::Num(1.0), Type::Number, span))), Type::Number, span);
        let update = TExpr::new(TExprKind::Assign(Place::Var(i_v), bx(inc)), Type::Number, span);
        let loop_stmt = TStmt::Loop { id, cond: Some(cond), test_after: false, update: Some(update), body: prologue };
        Some(vec![
            TStmt::Let(obj_v, Some(obj)),
            TStmt::Let(len_v, Some(len)),
            TStmt::Let(i_v, Some(TExpr::new(TExprKind::Num(0.0), Type::Number, span))),
            loop_stmt,
        ])
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
                // Evaluate the value once, before any change: `m.set(k,
                // (m.get(k) ?? 0) + 1)` must not see the key pushed without
                // its value (that read was out of bounds).
                let val_v = self.temp(v.clone());
                let val_r = TExpr::new(TExprKind::Var(val_v), v.clone(), span);
                let set_existing = self.arr_set_at(values.clone(), idx_r, val_r.clone(), span);
                let push_key = self.arr_push_discard(keys, key_r, span);
                let push_val = self.arr_push_discard(values, val_r, span);
                let if_stmt = TStmt::If(found, vec![set_existing], vec![push_key, push_val]);
                let body = vec![
                    TStmt::Let(obj_v, Some(o)),
                    TStmt::Let(key_v, Some(key)),
                    TStmt::Let(val_v, Some(val)),
                    TStmt::Let(idx_v, Some(search)),
                    if_stmt,
                ];
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
            // `m.forEach((value, key) => ...)` (JS order: value first): an
            // index loop over the `keys`/`values` arrays, the same shape
            // `kv_for_of` already builds for a bare `for…of`. No new
            // runtime function or codegen support is needed.
            "forEach" => {
                if args.len() != 1 {
                    wrong_arity(self, "forEach", 1);
                    return self.void_tail(span);
                }
                let (f, arity) = self.callback(&args[0], &[v.clone(), k.clone()], Some(Type::Void));
                let obj_v = self.temp(o.ty.clone());
                let obj_r = TExpr::new(TExprKind::Var(obj_v), o.ty.clone(), span);
                let keys = self.kv_field(&obj_r, sid, 0, span);
                let values = self.kv_field(&obj_r, sid, 1, span);
                let len = self.arr_len_of(keys.clone(), span);
                let len_v = self.temp(Type::Number);
                let len_r = TExpr::new(TExprKind::Var(len_v), Type::Number, span);
                let i_v = self.temp(Type::Number);
                let i_r = TExpr::new(TExprKind::Var(i_v), Type::Number, span);
                let f_v = self.temp(f.ty.clone());
                let f_r = TExpr::new(TExprKind::Var(f_v), f.ty.clone(), span);
                let val_at = self.arr_get_at(values, i_r.clone(), v.clone(), span);
                let key_at = self.arr_get_at(keys, i_r.clone(), k.clone(), span);
                let mut call_args = vec![val_at, key_at];
                call_args.truncate(arity);
                let call = TExpr::new(TExprKind::CallClosure(bx(f_r), call_args), Type::Void, span);
                let inc = TExpr::new(TExprKind::Num2(NumOp::Add, bx(i_r.clone()), bx(TExpr::new(TExprKind::Num(1.0), Type::Number, span))), Type::Number, span);
                let update = TExpr::new(TExprKind::Assign(Place::Var(i_v), bx(inc)), Type::Number, span);
                let cond = TExpr::new(TExprKind::Cmp(CmpOp::Lt, EqKind::F64, bx(i_r.clone()), bx(len_r)), Type::Bool, span);
                let id = self.prog.new_loop();
                let loop_stmt = TStmt::Loop { id, cond: Some(cond), test_after: false, update: Some(update), body: vec![TStmt::Expr(call)] };
                let prelude = vec![
                    TStmt::Let(obj_v, Some(o)),
                    TStmt::Let(f_v, Some(f)),
                    TStmt::Let(len_v, Some(len)),
                    TStmt::Let(i_v, Some(TExpr::new(TExprKind::Num(0.0), Type::Number, span))),
                    loop_stmt,
                ];
                TExpr::new(TExprKind::Block(prelude, bx(self.void_tail(span))), Type::Void, span)
            }
            // `.keys()`/`.values()` as a plain array (not only as a bare
            // `for…of` target, which `kv_for_of` already handles): a copy
            // of the backing array, so `[...m.keys()]` works through the
            // array literal's existing spread handling with no new
            // runtime function. `.entries()` is not included: there is no
            // tuple/array-of-pairs type to return it as (the `for…of
            // ([k, v] of m)` form still works, via `kv_for_of`).
            "keys" => {
                self.no_args(args, span);
                self.arr_copy_of(self.kv_field(&o, sid, 0, span), span)
            }
            "values" => {
                self.no_args(args, span);
                self.arr_copy_of(self.kv_field(&o, sid, 1, span), span)
            }
            // `.entries()` as a plain `[K, V][]` (a tuple is a 2-field
            // struct, see `tuple_struct`): an index loop that pushes one new
            // pair per entry. The pairs are copies, so a change to the
            // result does not change the map.
            "entries" => {
                self.no_args(args, span);
                let pair = self.tuple_struct(&[k.clone(), v.clone()]);
                let pair_ty = Type::Struct(pair);
                let res_ty = Type::Array(Box::new(pair_ty.clone()));
                let var = |id: VarId, ty: &Type| TExpr::new(TExprKind::Var(id), ty.clone(), span);
                let obj_v = self.temp(o.ty.clone());
                let obj_r = var(obj_v, &o.ty);
                let keys = self.kv_field(&obj_r, sid, 0, span);
                let values = self.kv_field(&obj_r, sid, 1, span);
                let res_v = self.temp(res_ty.clone());
                let len_v = self.temp(Type::Number);
                let i_v = self.temp(Type::Number);
                let i_r = var(i_v, &Type::Number);
                let lit = TExpr::new(
                    TExprKind::StructLit(pair, vec![self.arr_get_at(keys.clone(), i_r.clone(), k.clone(), span), self.arr_get_at(values, i_r.clone(), v.clone(), span)]),
                    pair_ty,
                    span,
                );
                let push = self.arr_push_discard(var(res_v, &res_ty), lit, span);
                let cond = TExpr::new(TExprKind::Cmp(CmpOp::Lt, EqKind::F64, bx(i_r.clone()), bx(var(len_v, &Type::Number))), Type::Bool, span);
                let inc = TExpr::new(TExprKind::Num2(NumOp::Add, bx(i_r), bx(self.num_lit(1.0, span))), Type::Number, span);
                let update = TExpr::new(TExprKind::Assign(Place::Var(i_v), bx(inc)), Type::Number, span);
                let id = self.prog.new_loop();
                let stmts = vec![
                    TStmt::Let(obj_v, Some(o)),
                    TStmt::Let(res_v, Some(TExpr::new(TExprKind::ArrayLit(Vec::new()), res_ty.clone(), span))),
                    TStmt::Let(len_v, Some(self.arr_len_of(keys, span))),
                    TStmt::Let(i_v, Some(self.num_lit(0.0, span))),
                    TStmt::Loop { id, cond: Some(cond), test_after: false, update: Some(update), body: vec![push] },
                ];
                TExpr::new(TExprKind::Block(stmts, bx(var(res_v, &res_ty))), res_ty, span)
            }
            _ => {
                self.err_help(
                    code::NO_PROPERTY,
                    prop_span,
                    format!("`Map` has no method `{prop}`"),
                    "use `get`, `set`, `has`, `delete`, `clear`, `forEach`, `keys`, `values` or `entries`",
                );
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
            // `s.forEach(v => ...)`: same shape as `Map.forEach`, over the
            // single `keys` array.
            "forEach" => {
                if args.len() != 1 {
                    self.err(code::ARG_COUNT, span, "`forEach` takes one function");
                    return self.void_tail(span);
                }
                let (f, arity) = self.callback(&args[0], &[t.clone()], Some(Type::Void));
                let obj_v = self.temp(o.ty.clone());
                let obj_r = TExpr::new(TExprKind::Var(obj_v), o.ty.clone(), span);
                let keys = self.kv_field(&obj_r, sid, 0, span);
                let len = self.arr_len_of(keys.clone(), span);
                let len_v = self.temp(Type::Number);
                let len_r = TExpr::new(TExprKind::Var(len_v), Type::Number, span);
                let i_v = self.temp(Type::Number);
                let i_r = TExpr::new(TExprKind::Var(i_v), Type::Number, span);
                let f_v = self.temp(f.ty.clone());
                let f_r = TExpr::new(TExprKind::Var(f_v), f.ty.clone(), span);
                let val_at = self.arr_get_at(keys, i_r.clone(), t.clone(), span);
                let mut call_args = vec![val_at];
                call_args.truncate(arity);
                let call = TExpr::new(TExprKind::CallClosure(bx(f_r), call_args), Type::Void, span);
                let inc = TExpr::new(TExprKind::Num2(NumOp::Add, bx(i_r.clone()), bx(TExpr::new(TExprKind::Num(1.0), Type::Number, span))), Type::Number, span);
                let update = TExpr::new(TExprKind::Assign(Place::Var(i_v), bx(inc)), Type::Number, span);
                let cond = TExpr::new(TExprKind::Cmp(CmpOp::Lt, EqKind::F64, bx(i_r.clone()), bx(len_r)), Type::Bool, span);
                let id = self.prog.new_loop();
                let loop_stmt = TStmt::Loop { id, cond: Some(cond), test_after: false, update: Some(update), body: vec![TStmt::Expr(call)] };
                let prelude = vec![
                    TStmt::Let(obj_v, Some(o)),
                    TStmt::Let(f_v, Some(f)),
                    TStmt::Let(len_v, Some(len)),
                    TStmt::Let(i_v, Some(TExpr::new(TExprKind::Num(0.0), Type::Number, span))),
                    loop_stmt,
                ];
                TExpr::new(TExprKind::Block(prelude, bx(self.void_tail(span))), Type::Void, span)
            }
            // `.keys()`/`.values()` as a plain array: both are the same
            // backing array for a `Set` (there is only one), copied so the
            // caller cannot mutate the `Set` through the result.
            "keys" | "values" => {
                self.no_args(args, span);
                self.arr_copy_of(self.kv_field(&o, sid, 0, span), span)
            }
            _ => {
                self.err_help(
                    code::NO_PROPERTY,
                    prop_span,
                    format!("`Set` has no method `{prop}`"),
                    "use `add`, `has`, `delete`, `clear`, `forEach`, `keys` or `values`",
                );
                TExpr::new(TExprKind::Null, Type::Error, span)
            }
        }
    }

    // -- Literals ---------------------------------------------------------

    fn array_lit(&mut self, elems: &[(bool, Expr)], expected: Option<&Type>, span: Span) -> TExpr {
        // `["a", 1]` where a tuple `[string, number]` is expected.
        let tuple = match expected {
            Some(Type::Struct(sid)) => Some(*sid),
            Some(Type::Nullable(inner)) => match &**inner {
                Type::Struct(sid) => Some(*sid),
                _ => None,
            },
            _ => None,
        };
        if let Some(sid) = tuple
            && let Some(shape) = self.tuple_shape(sid)
        {
            let n = shape.fixed.len();
            let head = &elems[..elems.len().min(n)];
            if elems.len() < shape.required || (shape.rest.is_none() && elems.len() > n) || head.iter().any(|(spread, _)| *spread) {
                let name = &self.prog.structs[sid as usize].name;
                let msg = match (&shape.rest, shape.required == n) {
                    (None, true) => format!("`{name}` needs exactly {n} element(s) and no spread"),
                    (None, false) => format!("`{name}` needs {} to {n} element(s) and no spread", shape.required),
                    (Some(_), _) => format!("`{name}` needs at least {} element(s), and a spread only after the first {n}", shape.required),
                };
                self.err(code::TYPE_MISMATCH, span, msg);
                return TExpr::new(TExprKind::Null, Type::Error, span);
            }
            let mut values = Vec::new();
            for (i, t) in shape.fixed.iter().enumerate() {
                match head.get(i) {
                    Some((_, e)) => {
                        let te = self.expr_with(e, t);
                        values.push(self.coerce(te, t));
                    }
                    None => values.push(self.coerce(TExpr::new(TExprKind::Null, Type::Null, span), t)),
                }
            }
            if let Some(rest) = &shape.rest {
                let aty = Type::Array(Box::new(rest.clone()));
                let tail = &elems[head.len()..];
                let rspan = tail.first().map(|(_, e)| e.span).unwrap_or(span);
                let arr = self.array_lit(tail, Some(&aty), rspan);
                values.push(self.coerce(arr, &aty));
            }
            return TExpr::new(TExprKind::StructLit(sid, values), Type::Struct(sid), span);
        }
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
                (x, y) if x == *y && x.repr() == crate::types::Repr::I32 => Some(EqKind::NullI32),
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
                let unwrapped = if !matches!(lt.ty, Type::Nullable(_)) {
                    TExpr::new(TExprKind::Coerce(Coercion::Retag, bx(read.clone())), inner, l.span)
                } else {
                    match inner.repr() {
                        crate::types::Repr::F64 => TExpr::new(TExprKind::Coerce(Coercion::UnboxNum, bx(read.clone())), inner, l.span),
                        crate::types::Repr::I32 => TExpr::new(TExprKind::Coerce(Coercion::UnboxI32, bx(read.clone())), inner, l.span),
                        _ => TExpr::new(TExprKind::Coerce(Coercion::Retag, bx(read.clone())), inner, l.span),
                    }
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
                    Type::Struct(sid) if self.tuple_shape(sid).is_some() => match self.tuple_index(sid, index)? {
                        TupleAt::Field(i, ty) => Some((Place::Field(bx(o), sid, i), ty)),
                        TupleAt::Rest(f, k, ty) => {
                            let arr = TExpr::new(TExprKind::Field(bx(o), sid, f), Type::Array(Box::new(ty.clone())), target.span);
                            Some((Place::Index(bx(arr), bx(self.num_lit(k as f64, target.span))), ty))
                        }
                    },
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
        // An assignment to a field (or an index, conservatively) drops any
        // member-path narrowing: nothing proves it wasn't the narrowed path
        // itself, or an alias of its root, that just changed.
        if !matches!(&place, Place::Var(_)) {
            self.invalidate_member_narrowing();
        }
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

    // -- JSON (plinth:core, SPEC.md §4.7) ----------------------------------
    //
    // `JSON.stringify` is generated per static type: no runtime schema, just
    // a tree of string concatenations and (for arrays and `Map`, whose
    // length is dynamic) a small loop. The runtime does only two small
    // jobs, in their own functions (HANDOFF.md §7 size policy): formatting
    // a number the JS way (`NaN`/`Infinity` -> `null`) and quoting/escaping
    // a string.

    /// Builds a `string` expression that serializes `value` as JSON.
    pub(super) fn json_stringify_value(&mut self, value: TExpr, span: Span) -> TExpr {
        match value.ty.clone() {
            Type::Number => TExpr::new(TExprKind::Rt("json_num_str", vec![value]), Type::String, span),
            Type::Int => {
                let n = TExpr::new(TExprKind::Coerce(Coercion::I32ToNum, bx(value)), Type::Number, span);
                TExpr::new(TExprKind::Rt("json_num_str", vec![n]), Type::String, span)
            }
            Type::Bool => TExpr::new(TExprKind::Coerce(Coercion::BoolToStr, bx(value)), Type::String, span),
            Type::String => TExpr::new(TExprKind::Rt("json_quote_str", vec![value]), Type::String, span),
            Type::StrLits(_) => {
                let s = TExpr::new(TExprKind::Coerce(Coercion::Retag, bx(value)), Type::String, span);
                TExpr::new(TExprKind::Rt("json_quote_str", vec![s]), Type::String, span)
            }
            Type::Null => TExpr::new(TExprKind::Str("null".into()), Type::String, span),
            Type::Nullable(inner) => self.json_stringify_nullable(value, &inner, span),
            Type::Array(elem) => self.json_stringify_array(value, &elem, span),
            Type::Struct(sid) => self.json_stringify_struct(value, sid, span),
            Type::Map(k, v) if *k == Type::String => self.json_stringify_map(value, &v, span),
            Type::Error => TExpr { ty: Type::Error, ..value },
            other => {
                let msg = format!("`JSON.stringify` does not support a value of type `{}`", self.show(&other));
                self.err(code::TYPE_MISMATCH, span, msg);
                TExpr::new(TExprKind::Str(String::new()), Type::Error, span)
            }
        }
    }

    fn json_stringify_nullable(&mut self, value: TExpr, inner: &Type, span: Span) -> TExpr {
        let nullable_ty = value.ty.clone();
        let v = self.temp(nullable_ty.clone());
        let v_r = TExpr::new(TExprKind::Var(v), nullable_ty, span);
        let is_null = TExpr::new(TExprKind::IsNull(bx(v_r.clone())), Type::Bool, span);
        let unboxed = match inner.repr() {
            crate::types::Repr::F64 => TExpr::new(TExprKind::Coerce(Coercion::UnboxNum, bx(v_r)), inner.clone(), span),
            crate::types::Repr::I32 => TExpr::new(TExprKind::Coerce(Coercion::UnboxI32, bx(v_r)), inner.clone(), span),
            _ => TExpr::new(TExprKind::Coerce(Coercion::Retag, bx(v_r)), inner.clone(), span),
        };
        let inner_str = self.json_stringify_value(unboxed, span);
        let null_lit = TExpr::new(TExprKind::Str("null".into()), Type::String, span);
        let cond = TExpr::new(TExprKind::Cond(bx(is_null), bx(null_lit), bx(inner_str)), Type::String, span);
        TExpr::new(TExprKind::Block(vec![TStmt::Let(v, Some(value))], bx(cond)), Type::String, span)
    }

    /// Struct fields are a fixed, known-at-compile-time list: no loop, just
    /// a chain of concatenations in declaration order.
    fn json_stringify_struct(&mut self, value: TExpr, sid: crate::types::StructId, span: Span) -> TExpr {
        let fields = self.prog.structs[sid as usize].fields.clone();
        // A tuple is a JSON array, as in JS.
        let tuple = self.tuple_shape(sid).is_some();
        let (open, close) = if tuple { ("[", "]") } else { ("{", "}") };
        let v = self.temp(value.ty.clone());
        let v_r = TExpr::new(TExprKind::Var(v), value.ty.clone(), span);
        let mut acc = TExpr::new(TExprKind::Str(open.into()), Type::String, span);
        for (i, f) in fields.iter().enumerate() {
            let prefix = match (tuple, i) {
                (true, 0) => String::new(),
                (true, _) => ",".into(),
                (false, 0) => format!("\"{}\":", f.name),
                (false, _) => format!(",\"{}\":", f.name),
            };
            let field_val = TExpr::new(TExprKind::Field(bx(v_r.clone()), sid, i as u32), f.ty.clone(), span);
            if tuple && f.name == "..." {
                // The rest elements of a tuple: `,` and the array's JSON
                // without its brackets, or nothing for an empty array.
                let len = self.arr_len_of(field_val.clone(), span);
                let arr_str = self.json_stringify_value(field_val, span);
                let inner = TExpr::new(TExprKind::Rt("str_slice", vec![arr_str, self.num_lit(1.0, span), self.num_lit(-1.0, span)]), Type::String, span);
                let comma = TExpr::new(TExprKind::Str(",".into()), Type::String, span);
                let piece = TExpr::new(TExprKind::Concat(bx(comma), bx(inner)), Type::String, span);
                let some = TExpr::new(TExprKind::Cmp(CmpOp::Gt, EqKind::F64, bx(len), bx(self.num_lit(0.0, span))), Type::Bool, span);
                let none = TExpr::new(TExprKind::Str(String::new()), Type::String, span);
                let piece = TExpr::new(TExprKind::Cond(bx(some), bx(piece), bx(none)), Type::String, span);
                acc = TExpr::new(TExprKind::Concat(bx(acc), bx(piece)), Type::String, span);
                continue;
            }
            let field_str = self.json_stringify_value(field_val, span);
            let prefix_e = TExpr::new(TExprKind::Str(prefix), Type::String, span);
            let piece = TExpr::new(TExprKind::Concat(bx(prefix_e), bx(field_str)), Type::String, span);
            acc = TExpr::new(TExprKind::Concat(bx(acc), bx(piece)), Type::String, span);
        }
        let close = TExpr::new(TExprKind::Str(close.into()), Type::String, span);
        let result = TExpr::new(TExprKind::Concat(bx(acc), bx(close)), Type::String, span);
        TExpr::new(TExprKind::Block(vec![TStmt::Let(v, Some(value))], bx(result)), Type::String, span)
    }

    /// `[e0,e1,...]`: a loop, since the length is only known at run time.
    fn json_stringify_array(&mut self, value: TExpr, elem_ty: &Type, span: Span) -> TExpr {
        let id = self.prog.new_loop();
        let arr_v = self.temp(value.ty.clone());
        let arr_r = TExpr::new(TExprKind::Var(arr_v), value.ty.clone(), span);
        let len = self.arr_len_of(arr_r.clone(), span);
        let len_v = self.temp(Type::Number);
        let len_r = TExpr::new(TExprKind::Var(len_v), Type::Number, span);
        let i_v = self.temp(Type::Number);
        let i_r = TExpr::new(TExprKind::Var(i_v), Type::Number, span);
        let acc_v = self.temp(Type::String);
        let acc_r = TExpr::new(TExprKind::Var(acc_v), Type::String, span);

        let elem = self.arr_get_at(arr_r, i_r.clone(), elem_ty.clone(), span);
        let elem_str = self.json_stringify_value(elem, span);
        let append_elem = self.json_append(acc_v, elem_str, span);
        let append_comma = self.json_append(acc_v, TExpr::new(TExprKind::Str(",".into()), Type::String, span), span);
        let sep_cond = TExpr::new(TExprKind::Cmp(CmpOp::Gt, EqKind::F64, bx(i_r.clone()), bx(TExpr::new(TExprKind::Num(0.0), Type::Number, span))), Type::Bool, span);
        let if_sep = TStmt::If(sep_cond, vec![append_comma], Vec::new());

        let cond = TExpr::new(TExprKind::Cmp(CmpOp::Lt, EqKind::F64, bx(i_r.clone()), bx(len_r)), Type::Bool, span);
        let inc = TExpr::new(TExprKind::Num2(NumOp::Add, bx(i_r), bx(TExpr::new(TExprKind::Num(1.0), Type::Number, span))), Type::Number, span);
        let update = TExpr::new(TExprKind::Assign(Place::Var(i_v), bx(inc)), Type::Number, span);
        let loop_stmt = TStmt::Loop { id, cond: Some(cond), test_after: false, update: Some(update), body: vec![if_sep, append_elem] };

        let open = TExpr::new(TExprKind::Str("[".into()), Type::String, span);
        let close = TExpr::new(TExprKind::Str("]".into()), Type::String, span);
        let result = TExpr::new(TExprKind::Concat(bx(acc_r), bx(close)), Type::String, span);
        let body = vec![
            TStmt::Let(arr_v, Some(value)),
            TStmt::Let(len_v, Some(len)),
            TStmt::Let(i_v, Some(TExpr::new(TExprKind::Num(0.0), Type::Number, span))),
            TStmt::Let(acc_v, Some(open)),
            loop_stmt,
        ];
        TExpr::new(TExprKind::Block(body, bx(result)), Type::String, span)
    }

    /// `Map<string, V>` as a JSON object: same loop shape as an array.
    fn json_stringify_map(&mut self, value: TExpr, v_ty: &Type, span: Span) -> TExpr {
        let sid = self.map_struct(&Type::String, v_ty);
        let id = self.prog.new_loop();
        let obj_v = self.temp(value.ty.clone());
        let obj_r = TExpr::new(TExprKind::Var(obj_v), value.ty.clone(), span);
        let keys = self.kv_field(&obj_r, sid, 0, span);
        let values = self.kv_field(&obj_r, sid, 1, span);
        let len = self.arr_len_of(keys.clone(), span);
        let len_v = self.temp(Type::Number);
        let len_r = TExpr::new(TExprKind::Var(len_v), Type::Number, span);
        let i_v = self.temp(Type::Number);
        let i_r = TExpr::new(TExprKind::Var(i_v), Type::Number, span);
        let acc_v = self.temp(Type::String);
        let acc_r = TExpr::new(TExprKind::Var(acc_v), Type::String, span);

        let key = self.arr_get_at(keys, i_r.clone(), Type::String, span);
        let key_str = TExpr::new(TExprKind::Rt("json_quote_str", vec![key]), Type::String, span);
        let val = self.arr_get_at(values, i_r.clone(), v_ty.clone(), span);
        let val_str = self.json_stringify_value(val, span);
        let colon = TExpr::new(TExprKind::Str(":".into()), Type::String, span);
        let entry = TExpr::new(
            TExprKind::Concat(bx(TExpr::new(TExprKind::Concat(bx(key_str), bx(colon)), Type::String, span)), bx(val_str)),
            Type::String,
            span,
        );
        let append_entry = self.json_append(acc_v, entry, span);
        let append_comma = self.json_append(acc_v, TExpr::new(TExprKind::Str(",".into()), Type::String, span), span);
        let sep_cond = TExpr::new(TExprKind::Cmp(CmpOp::Gt, EqKind::F64, bx(i_r.clone()), bx(TExpr::new(TExprKind::Num(0.0), Type::Number, span))), Type::Bool, span);
        let if_sep = TStmt::If(sep_cond, vec![append_comma], Vec::new());

        let cond = TExpr::new(TExprKind::Cmp(CmpOp::Lt, EqKind::F64, bx(i_r.clone()), bx(len_r)), Type::Bool, span);
        let inc = TExpr::new(TExprKind::Num2(NumOp::Add, bx(i_r), bx(TExpr::new(TExprKind::Num(1.0), Type::Number, span))), Type::Number, span);
        let update = TExpr::new(TExprKind::Assign(Place::Var(i_v), bx(inc)), Type::Number, span);
        let loop_stmt = TStmt::Loop { id, cond: Some(cond), test_after: false, update: Some(update), body: vec![if_sep, append_entry] };

        let open = TExpr::new(TExprKind::Str("{".into()), Type::String, span);
        let close = TExpr::new(TExprKind::Str("}".into()), Type::String, span);
        let result = TExpr::new(TExprKind::Concat(bx(acc_r), bx(close)), Type::String, span);
        let body = vec![
            TStmt::Let(obj_v, Some(value)),
            TStmt::Let(len_v, Some(len)),
            TStmt::Let(i_v, Some(TExpr::new(TExprKind::Num(0.0), Type::Number, span))),
            TStmt::Let(acc_v, Some(open)),
            loop_stmt,
        ];
        TExpr::new(TExprKind::Block(body, bx(result)), Type::String, span)
    }

    /// `acc = acc + piece;`
    fn json_append(&self, acc_v: VarId, piece: TExpr, span: Span) -> TStmt {
        let acc_r = TExpr::new(TExprKind::Var(acc_v), Type::String, span);
        let sum = TExpr::new(TExprKind::Concat(bx(acc_r), bx(piece)), Type::String, span);
        TStmt::Expr(TExpr::new(TExprKind::Assign(Place::Var(acc_v), bx(sum)), Type::String, span))
    }

    // -- JSON.parse<T> (plinth:core, SPEC.md §4.7) -------------------------
    //
    // The decoder is generated per static type `T`, like `json_stringify_value`
    // but reversed: it reads from the runtime's cursor (`crates/plinth-rt/src/json.rs`)
    // and keeps going after a failure (every `json_*` read then returns a
    // safe default), so the generated TIR never branches on failure itself.
    // Only the very end checks `json_finish()` and picks `null` if anything
    // along the way failed.

    /// Builds the `T | null` expression for `JSON.parse<T>(text)`.
    pub(super) fn json_parse_value(&mut self, text: TExpr, ty: &Type, span: Span) -> TExpr {
        if ty.is_error() {
            return TExpr::new(TExprKind::Null, Type::Error, span);
        }
        let result_ty = match ty {
            Type::Nullable(_) => ty.clone(),
            _ => Type::Nullable(Box::new(ty.clone())),
        };
        let begin = TExpr::new(TExprKind::Rt("json_begin", vec![text]), Type::Void, span);
        let decoded = self.json_decode(ty, span);
        let decoded_v = self.temp(ty.clone());
        let decoded_r = TExpr::new(TExprKind::Var(decoded_v), ty.clone(), span);
        let finish_ok = TExpr::new(TExprKind::Rt("json_finish", Vec::new()), Type::Bool, span);
        let success = match ty {
            Type::Nullable(_) => decoded_r,
            _ => self.json_box_nullable(decoded_r, ty, span),
        };
        let failure = TExpr::new(TExprKind::Null, result_ty.clone(), span);
        let tail = TExpr::new(TExprKind::Cond(bx(finish_ok), bx(success), bx(failure)), result_ty.clone(), span);
        let body = vec![TStmt::Expr(begin), TStmt::Let(decoded_v, Some(decoded))];
        TExpr::new(TExprKind::Block(body, bx(tail)), result_ty, span)
    }

    /// `inner` -> `inner | null` (a value already in hand, not read from JSON).
    fn json_box_nullable(&self, value: TExpr, inner: &Type, span: Span) -> TExpr {
        let nullable_ty = Type::Nullable(Box::new(inner.clone()));
        match inner.repr() {
            crate::types::Repr::F64 => TExpr::new(TExprKind::Coerce(Coercion::BoxNum, bx(value)), nullable_ty, span),
            crate::types::Repr::I32 => TExpr::new(TExprKind::Coerce(Coercion::BoxI32, bx(value)), nullable_ty, span),
            _ => TExpr::new(TExprKind::Coerce(Coercion::Retag, bx(value)), nullable_ty, span),
        }
    }

    /// Reads one value of type `ty` from the cursor.
    fn json_decode(&mut self, ty: &Type, span: Span) -> TExpr {
        match ty {
            Type::Number => TExpr::new(TExprKind::Rt("json_read_num", Vec::new()), Type::Number, span),
            Type::Int => TExpr::new(TExprKind::Rt("json_read_int", Vec::new()), Type::Int, span),
            Type::Bool => TExpr::new(TExprKind::Rt("json_read_bool", Vec::new()), Type::Bool, span),
            Type::String => TExpr::new(TExprKind::Rt("json_read_str", Vec::new()), Type::String, span),
            Type::StrLits(lits) => self.json_decode_str_lits(lits.clone(), span),
            Type::Null => {
                let read = TExpr::new(TExprKind::Rt("json_read_null", Vec::new()), Type::Void, span);
                let null_v = TExpr::new(TExprKind::Null, Type::Null, span);
                TExpr::new(TExprKind::Block(vec![TStmt::Expr(read)], bx(null_v)), Type::Null, span)
            }
            Type::Nullable(inner) => self.json_decode_nullable(inner, span),
            Type::Array(elem) => self.json_decode_array(elem, span),
            Type::Struct(sid) if self.tuple_shape(*sid).is_some() => {
                let msg = format!("`JSON.parse` does not support the tuple type `{}`", self.show(ty));
                self.err_help(code::TYPE_MISMATCH, span, msg, "parse an interface with named fields");
                TExpr::new(TExprKind::Null, Type::Error, span)
            }
            Type::Struct(sid) => self.json_decode_struct(*sid, span),
            Type::Map(k, v) if **k == Type::String => self.json_decode_map(v, span),
            Type::Error => TExpr::new(TExprKind::Null, Type::Error, span),
            other => {
                let msg = format!("`JSON.parse` does not support a value of type `{}`", self.show(other));
                self.err(code::TYPE_MISMATCH, span, msg);
                TExpr::new(TExprKind::Null, Type::Error, span)
            }
        }
    }

    fn json_decode_str_lits(&mut self, lits: Rc<[String]>, span: Span) -> TExpr {
        let v = self.temp(Type::String);
        let v_r = TExpr::new(TExprKind::Var(v), Type::String, span);
        let read = TExpr::new(TExprKind::Rt("json_read_str", Vec::new()), Type::String, span);
        let mut cond: Option<TExpr> = None;
        for lit in lits.iter() {
            let eq = TExpr::new(
                TExprKind::Cmp(CmpOp::Eq, EqKind::Str, bx(v_r.clone()), bx(TExpr::new(TExprKind::Str(lit.clone()), Type::String, span))),
                Type::Bool,
                span,
            );
            cond = Some(match cond {
                Some(c) => TExpr::new(TExprKind::Or(bx(c), bx(eq)), Type::Bool, span),
                None => eq,
            });
        }
        let matched = cond.unwrap_or_else(|| TExpr::new(TExprKind::Bool(false), Type::Bool, span));
        let not_matched = TExpr::new(TExprKind::Not(bx(matched)), Type::Bool, span);
        let fail_call = TExpr::new(TExprKind::Rt("json_fail", Vec::new()), Type::Void, span);
        let guard = TStmt::If(not_matched, vec![TStmt::Expr(fail_call)], Vec::new());
        let result = TExpr::new(TExprKind::Coerce(Coercion::Retag, bx(v_r)), Type::StrLits(lits.clone()), span);
        TExpr::new(TExprKind::Block(vec![TStmt::Let(v, Some(read)), guard], bx(result)), Type::StrLits(lits), span)
    }

    fn json_decode_nullable(&mut self, inner: &Type, span: Span) -> TExpr {
        let kind = TExpr::new(TExprKind::Rt("json_peek_kind", Vec::new()), Type::Int, span);
        let is_null = TExpr::new(
            TExprKind::Cmp(CmpOp::Eq, EqKind::I32, bx(kind), bx(TExpr::new(TExprKind::Num(0.0), Type::Int, span))),
            Type::Bool,
            span,
        );
        let nullable_ty = Type::Nullable(Box::new(inner.clone()));
        let consume = TExpr::new(TExprKind::Rt("json_read_null", Vec::new()), Type::Void, span);
        let null_v = TExpr::new(TExprKind::Null, nullable_ty.clone(), span);
        let null_branch = TExpr::new(TExprKind::Block(vec![TStmt::Expr(consume)], bx(null_v)), nullable_ty.clone(), span);
        let inner_val = self.json_decode(inner, span);
        let inner_branch = self.json_box_nullable(inner_val, inner, span);
        TExpr::new(TExprKind::Cond(bx(is_null), bx(null_branch), bx(inner_branch)), nullable_ty, span)
    }

    fn json_decode_array(&mut self, elem_ty: &Type, span: Span) -> TExpr {
        let arr_ty = Type::Array(Box::new(elem_ty.clone()));
        let arr_v = self.temp(arr_ty.clone());
        let arr_r = TExpr::new(TExprKind::Var(arr_v), arr_ty.clone(), span);
        let begin = TExpr::new(TExprKind::Rt("json_arr_begin", Vec::new()), Type::Void, span);
        let next = TExpr::new(TExprKind::Rt("json_arr_next", Vec::new()), Type::Bool, span);
        let elem_val = self.json_decode(elem_ty, span);
        let push = self.arr_push_discard(arr_r.clone(), elem_val, span);
        let id = self.prog.new_loop();
        let loop_stmt = TStmt::Loop { id, cond: Some(next), test_after: false, update: None, body: vec![push] };
        let body = vec![
            TStmt::Let(arr_v, Some(TExpr::new(TExprKind::ArrayLit(Vec::new()), arr_ty.clone(), span))),
            TStmt::Expr(begin),
            loop_stmt,
        ];
        TExpr::new(TExprKind::Block(body, bx(arr_r)), arr_ty, span)
    }

    /// An object with a known shape: a `while (true)` loop over the keys
    /// that the cursor reports, matching each against the field names. An
    /// unknown key (or any key, for an empty-struct edge case) is skipped.
    fn json_decode_struct(&mut self, sid: crate::types::StructId, span: Span) -> TExpr {
        let fields = self.prog.structs[sid as usize].fields.clone();
        let field_vars: Vec<VarId> = fields.iter().map(|f| self.temp(f.ty.clone())).collect();
        let seen_vars: Vec<VarId> = fields.iter().map(|_| self.temp(Type::Bool)).collect();

        let mut stmts = Vec::new();
        for (i, f) in fields.iter().enumerate() {
            let default = self.json_default(&f.ty, span);
            stmts.push(TStmt::Let(field_vars[i], Some(default)));
            stmts.push(TStmt::Let(seen_vars[i], Some(TExpr::new(TExprKind::Bool(false), Type::Bool, span))));
        }
        stmts.push(TStmt::Expr(TExpr::new(TExprKind::Rt("json_obj_begin", Vec::new()), Type::Void, span)));

        let key_v = self.temp(Type::String);
        let key_r = TExpr::new(TExprKind::Var(key_v), Type::String, span);
        let next_key = TExpr::new(TExprKind::Rt("json_obj_next_key", Vec::new()), Type::String, span);
        let assign_key = TStmt::Expr(TExpr::new(TExprKind::Assign(Place::Var(key_v), bx(next_key)), Type::String, span));
        let is_none = TExpr::new(TExprKind::IsNull(bx(key_r.clone())), Type::Bool, span);
        let break_if_none = TStmt::If(is_none, vec![TStmt::Break], Vec::new());

        let mut chain: Vec<TStmt> = vec![TStmt::Expr(TExpr::new(TExprKind::Rt("json_skip_value", Vec::new()), Type::Void, span))];
        for (i, f) in fields.iter().enumerate().rev() {
            let eq = TExpr::new(
                TExprKind::Cmp(CmpOp::Eq, EqKind::Str, bx(key_r.clone()), bx(TExpr::new(TExprKind::Str(f.name.clone()), Type::String, span))),
                Type::Bool,
                span,
            );
            let decode_val = self.json_decode(&f.ty, span);
            let assign_field = TStmt::Expr(TExpr::new(TExprKind::Assign(Place::Var(field_vars[i]), bx(decode_val)), f.ty.clone(), span));
            let mark_seen = TStmt::Expr(TExpr::new(
                TExprKind::Assign(Place::Var(seen_vars[i]), bx(TExpr::new(TExprKind::Bool(true), Type::Bool, span))),
                Type::Bool,
                span,
            ));
            chain = vec![TStmt::If(eq, vec![assign_field, mark_seen], chain)];
        }

        let id = self.prog.new_loop();
        let mut loop_body = vec![assign_key, break_if_none];
        loop_body.extend(chain);
        stmts.push(TStmt::Let(key_v, Some(TExpr::new(TExprKind::Null, Type::String, span))));
        stmts.push(TStmt::Loop { id, cond: None, test_after: false, update: None, body: loop_body });

        for (i, f) in fields.iter().enumerate() {
            if f.optional {
                continue;
            }
            let not_seen = TExpr::new(TExprKind::Not(bx(TExpr::new(TExprKind::Var(seen_vars[i]), Type::Bool, span))), Type::Bool, span);
            let fail_call = TExpr::new(TExprKind::Rt("json_fail", Vec::new()), Type::Void, span);
            stmts.push(TStmt::If(not_seen, vec![TStmt::Expr(fail_call)], Vec::new()));
        }

        let values: Vec<TExpr> = field_vars.iter().zip(fields.iter()).map(|(v, f)| TExpr::new(TExprKind::Var(*v), f.ty.clone(), span)).collect();
        let lit = TExpr::new(TExprKind::StructLit(sid, values), Type::Struct(sid), span);
        TExpr::new(TExprKind::Block(stmts, bx(lit)), Type::Struct(sid), span)
    }

    /// `Map<string, V>` from a JSON object: the same key loop as a struct,
    /// but every key is accepted and pushed into the keys/values arrays.
    fn json_decode_map(&mut self, v_ty: &Type, span: Span) -> TExpr {
        let sid = self.map_struct(&Type::String, v_ty);
        let map_ty = Type::Map(Box::new(Type::String), Box::new(v_ty.clone()));
        let keys_ty = Type::Array(Box::new(Type::String));
        let vals_ty = Type::Array(Box::new(v_ty.clone()));
        let keys_v = self.temp(keys_ty.clone());
        let vals_v = self.temp(vals_ty.clone());
        let keys_r = TExpr::new(TExprKind::Var(keys_v), keys_ty.clone(), span);
        let vals_r = TExpr::new(TExprKind::Var(vals_v), vals_ty.clone(), span);

        let begin = TExpr::new(TExprKind::Rt("json_obj_begin", Vec::new()), Type::Void, span);
        let key_v = self.temp(Type::String);
        let key_r = TExpr::new(TExprKind::Var(key_v), Type::String, span);
        let next_key = TExpr::new(TExprKind::Rt("json_obj_next_key", Vec::new()), Type::String, span);
        let assign_key = TStmt::Expr(TExpr::new(TExprKind::Assign(Place::Var(key_v), bx(next_key)), Type::String, span));
        let is_none = TExpr::new(TExprKind::IsNull(bx(key_r.clone())), Type::Bool, span);
        let break_if_none = TStmt::If(is_none, vec![TStmt::Break], Vec::new());
        let val = self.json_decode(v_ty, span);
        let push_key = self.arr_push_discard(keys_r.clone(), key_r.clone(), span);
        let push_val = self.arr_push_discard(vals_r.clone(), val, span);

        let id = self.prog.new_loop();
        let loop_stmt = TStmt::Loop { id, cond: None, test_after: false, update: None, body: vec![assign_key, break_if_none, push_key, push_val] };
        let body = vec![
            TStmt::Let(keys_v, Some(TExpr::new(TExprKind::ArrayLit(Vec::new()), keys_ty, span))),
            TStmt::Let(vals_v, Some(TExpr::new(TExprKind::ArrayLit(Vec::new()), vals_ty, span))),
            TStmt::Let(key_v, Some(TExpr::new(TExprKind::Null, Type::String, span))),
            TStmt::Expr(begin),
            loop_stmt,
        ];
        let lit = TExpr::new(TExprKind::StructLit(sid, vec![keys_r, vals_r]), map_ty.clone(), span);
        TExpr::new(TExprKind::Block(body, bx(lit)), map_ty, span)
    }

    /// A safe placeholder value of type `ty`, used to initialize a struct
    /// field before the decoder has seen it (and, for a missing required
    /// field, never will: `json_fail` already marked the parse failed, and
    /// the caller discards the whole result in favor of `null`).
    fn json_default(&mut self, ty: &Type, span: Span) -> TExpr {
        match ty {
            Type::Number => TExpr::new(TExprKind::Num(0.0), Type::Number, span),
            Type::Int => TExpr::new(TExprKind::Num(0.0), Type::Int, span),
            Type::Bool => TExpr::new(TExprKind::Bool(false), Type::Bool, span),
            Type::String => TExpr::new(TExprKind::Str(String::new()), Type::String, span),
            Type::StrLits(lits) => {
                let first = lits.first().cloned().unwrap_or_default();
                let s = TExpr::new(TExprKind::Str(first), Type::String, span);
                TExpr::new(TExprKind::Coerce(Coercion::Retag, bx(s)), ty.clone(), span)
            }
            Type::Nullable(_) => TExpr::new(TExprKind::Null, ty.clone(), span),
            Type::Array(elem) => TExpr::new(TExprKind::ArrayLit(Vec::new()), Type::Array(elem.clone()), span),
            Type::Struct(sid) => {
                let fields = self.prog.structs[*sid as usize].fields.clone();
                let values = fields.iter().map(|f| self.json_default(&f.ty, span)).collect();
                TExpr::new(TExprKind::StructLit(*sid, values), ty.clone(), span)
            }
            Type::Map(k, v) => {
                let sid = self.map_struct(k, v);
                let ka = TExpr::new(TExprKind::ArrayLit(Vec::new()), Type::Array(k.clone()), span);
                let va = TExpr::new(TExprKind::ArrayLit(Vec::new()), Type::Array(v.clone()), span);
                TExpr::new(TExprKind::StructLit(sid, vec![ka, va]), ty.clone(), span)
            }
            _ => TExpr::new(TExprKind::Null, ty.clone(), span),
        }
    }
}

/// Where `t[k]` of a tuple is: a fixed element (field, type), or element
/// `k` of the rest array (its field, `k`, the element type).
pub(super) enum TupleAt {
    Field(u32, Type),
    Rest(u32, u32, Type),
}
