//! Lowers JSX and reactivity (SPEC.md §7.2) into runtime calls.
//!
//! - A control becomes `node(kind)` and prop calls. A dynamic prop (one
//!   whose expression calls a function, so it may read a signal) becomes an
//!   effect that sets the prop again when its dependencies change.
//! - An expression child becomes a region: an effect that replaces one
//!   child slot.
//! - `List` becomes the runtime's keyed reconciler.
//! - Signals, computed values and effects become runtime calls; closures
//!   that the runtime calls get a thunk (`ThunkOf`).
//!
//! It also makes the `main` function: the module initializers, then the
//! screens.

use crate::diag::Span;
use crate::tir::*;
use crate::types::{FuncType, Repr, Type};
use plinth_protocol::prop;
use std::rc::Rc;

pub fn lower(prog: &mut Program, dev: bool) -> FuncId {
    let n = prog.funcs.len();
    for fid in 0..n as FuncId {
        let body = std::mem::take(&mut prog.funcs[fid as usize].body);
        let mut cx = Cx { prog, func: fid, loops: Vec::new(), dev };
        let body = cx.stmts(body);
        prog.funcs[fid as usize].body = body;
    }
    make_main(prog)
}

/// Hot reload (SPEC.md §13): the wire shape of a module-level signal's
/// value type, or `None` when the type is not one `sig_register`/
/// `sig_restore` understand (an array, a struct, a union, ...). Those
/// signals are simply not registered, so they always start fresh; v1
/// covers the common scalar cases only (see HANDOFF.md-style note in
/// `reactive.rs`).
fn shape_code(ty: &Type) -> Option<u32> {
    match ty {
        Type::Number => Some(1),
        Type::Int => Some(2),
        Type::Bool => Some(3),
        Type::String | Type::StrLits(_) => Some(4),
        Type::Nullable(inner) => shape_code(inner).map(|c| c + 10),
        _ => None,
    }
}

/// A stable 32-bit hash of a module-level signal's key (module index +
/// declaration name, SPEC.md §13). FNV-1a: small, and the compiler is a
/// `std` crate, so this has nothing to do with the `plinth-rt` size budget.
fn fnv1a(s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

fn make_main(prog: &mut Program) -> FuncId {
    let span = Span::default();
    let mut body = Vec::new();
    for &init in &prog.module_inits {
        body.push(TStmt::Expr(TExpr::new(TExprKind::Call(init, Vec::new()), Type::Void, span)));
    }
    let main = prog.new_func(FuncDef {
        name: "<main>".into(),
        kind: FuncKind::TopLevel,
        params: Vec::new(),
        ret: Type::Void,
        body: Vec::new(),
        span,
    });
    let screens = prog.screens.clone();
    for (i, s) in screens.iter().enumerate() {
        let node = prog.new_var(VarInfo {
            name: format!("$screen{i}"),
            ty: Type::Element,
            owner: main,
            mutable: false,
            module: None,
            in_loop: None,
            captured: false,
        });
        let call = TExpr::new(TExprKind::Call(s.component, Vec::new()), Type::Element, span);
        body.push(TStmt::Let(node, Some(call)));
        let read = TExpr::new(TExprKind::Var(node), Type::Element, span);
        if let Some(icon) = &s.icon {
            body.push(rt_stmt("prop_str", vec![read.clone(), i32c(prop::ICON as i32), TExpr::new(TExprKind::Str(icon.clone()), Type::String, span)]));
        }
        body.push(rt_stmt("set_root", vec![i32c(i as i32), read]));
        if s.primary {
            body.push(rt_stmt("mark_primary", vec![i32c(i as i32)]));
        }
    }
    prog.funcs[main as usize].body = body;
    main
}

fn i32c(v: i32) -> TExpr {
    TExpr::new(TExprKind::Num(v as f64), Type::Bool, Span::default())
}

fn rt(name: &'static str, args: Vec<TExpr>, ty: Type) -> TExpr {
    TExpr::new(TExprKind::Rt(name, args), ty, Span::default())
}

fn rt_stmt(name: &'static str, args: Vec<TExpr>) -> TStmt {
    TStmt::Expr(rt(name, args, Type::Void))
}

/// The runtime function for a signal operation on a value of `ty`.
fn by_repr(ty: &Type, f64_name: &'static str, i32_name: &'static str, ref_name: &'static str) -> &'static str {
    match ty.repr() {
        Repr::F64 => f64_name,
        Repr::Ref => ref_name,
        _ => i32_name,
    }
}

/// True if evaluating `e` may read reactive state: it calls a function or
/// reads a signal. Such a prop or child must be an effect.
fn is_dynamic(e: &TExpr) -> bool {
    let mut dynamic = false;
    visit_expr(e, &mut |x| {
        if matches!(
            x.kind,
            TExprKind::Call(..)
                | TExprKind::CallClosure(..)
                | TExprKind::SignalGet(_)
                | TExprKind::ComputedGet(_)
                | TExprKind::ArrayHof { .. }
                | TExprKind::Rt("sig_get_f64" | "sig_get_i32" | "comp_get_f64" | "comp_get_i32", _)
        ) {
            dynamic = true;
        }
    });
    dynamic
}

/// Visits an expression tree without entering closures.
fn visit_expr(e: &TExpr, f: &mut dyn FnMut(&TExpr)) {
    f(e);
    let mut go = |x: &TExpr| visit_expr(x, f);
    match &e.kind {
        TExprKind::Assign(p, v) => {
            match p {
                Place::Var(_) => {}
                Place::Field(o, ..) => go(o),
                Place::Index(a, i) => {
                    go(a);
                    go(i);
                }
            }
            go(v);
        }
        TExprKind::Field(o, ..)
        | TExprKind::Neg(o)
        | TExprKind::Not(o)
        | TExprKind::IsNull(o)
        | TExprKind::Coerce(_, o)
        | TExprKind::SignalNew(o)
        | TExprKind::SignalGet(o)
        | TExprKind::SignalPeek(o)
        | TExprKind::ComputedNew(o)
        | TExprKind::ComputedGet(o)
        | TExprKind::EffectNew(o)
        | TExprKind::UnionTag(o) => go(o),
        TExprKind::UnionIs(o, _) => go(o),
        TExprKind::Index(a, b)
        | TExprKind::Num2(_, a, b)
        | TExprKind::Int2(_, a, b)
        | TExprKind::Cmp(_, _, a, b)
        | TExprKind::StrCmp(_, a, b)
        | TExprKind::Concat(a, b)
        | TExprKind::And(a, b)
        | TExprKind::Or(a, b)
        | TExprKind::SignalSet(a, b) => {
            go(a);
            go(b);
        }
        TExprKind::Cond(a, b, c) => {
            go(a);
            go(b);
            go(c);
        }
        TExprKind::Call(_, args) | TExprKind::Rt(_, args) | TExprKind::MathOp(_, args) | TExprKind::StructLit(_, args) => {
            for a in args {
                go(a);
            }
        }
        TExprKind::CallClosure(c, args) => {
            go(c);
            for a in args {
                go(a);
            }
        }
        TExprKind::ArrayLit(items) => {
            for (_, a) in items {
                go(a);
            }
        }
        TExprKind::ArrayHof { arr, f: cb, .. } => {
            go(arr);
            go(cb);
        }
        TExprKind::ArraySearch { arr, value, .. } => {
            go(arr);
            go(value);
        }
        TExprKind::Block(stmts, v) => {
            for s in stmts {
                visit_stmt(s, &mut go);
            }
            go(v);
        }
        TExprKind::Jsx(_) => {
            // JSX in an expression creates nodes: treat it as dynamic.
            f(&TExpr::new(TExprKind::CallClosure(Box::new(e.clone()), Vec::new()), Type::Void, e.span));
        }
        TExprKind::TimerNew(ms, _, cb) => {
            go(ms);
            go(cb);
        }
        _ => {}
    }
}

fn visit_stmt(s: &TStmt, go: &mut dyn FnMut(&TExpr)) {
    match s {
        TStmt::Let(_, Some(e)) | TStmt::Expr(e) | TStmt::Return(Some(e)) | TStmt::Throw(e) => go(e),
        TStmt::If(c, a, b) => {
            go(c);
            for x in a.iter().chain(b) {
                visit_stmt(x, go);
            }
        }
        TStmt::Loop { cond, update, body, .. } => {
            if let Some(c) = cond {
                go(c);
            }
            if let Some(u) = update {
                go(u);
            }
            for x in body {
                visit_stmt(x, go);
            }
        }
        TStmt::ForOf { arr, body, .. } => {
            go(arr);
            for x in body {
                visit_stmt(x, go);
            }
        }
        TStmt::Switch { disc, cases, .. } => {
            go(disc);
            for (t, b) in cases {
                if let Some(t) = t {
                    go(t);
                }
                for x in b {
                    visit_stmt(x, go);
                }
            }
        }
        TStmt::Block(b) => {
            for x in b {
                visit_stmt(x, go);
            }
        }
        _ => {}
    }
}

struct Cx<'p> {
    prog: &'p mut Program,
    func: FuncId,
    loops: Vec<LoopId>,
    /// Hot reload (SPEC.md §13): emit `sig_register` after a module-level
    /// `signal(...)` whose value has a shape `sig_register`/`sig_restore`
    /// understand.
    dev: bool,
}

impl Cx<'_> {
    fn stmts(&mut self, stmts: Vec<TStmt>) -> Vec<TStmt> {
        stmts.into_iter().map(|s| self.stmt(s)).collect()
    }

    fn stmt(&mut self, s: TStmt) -> TStmt {
        match s {
            TStmt::Let(v, Some(e)) if self.dev && self.is_registrable_module_signal(v) => {
                let span = e.span;
                let lowered = self.expr(e);
                let module_idx = match self.prog.funcs[self.func as usize].kind {
                    FuncKind::ModuleInit(m) => m,
                    _ => 0,
                };
                let shape = match &self.prog.vars[v as usize].ty {
                    Type::Signal(inner) => shape_code(inner).expect("checked by is_registrable_module_signal"),
                    _ => unreachable!(),
                };
                let key = fnv1a(&format!("{module_idx}:{}", self.prog.vars[v as usize].name));
                let read = TExpr::new(TExprKind::Var(v), self.prog.vars[v as usize].ty.clone(), span);
                TStmt::Block(vec![
                    TStmt::Let(v, Some(lowered)),
                    rt_stmt("sig_register", vec![read, i32c(key as i32), i32c(shape as i32)]),
                ])
            }
            TStmt::Let(v, e) => TStmt::Let(v, e.map(|e| self.expr(e))),
            TStmt::Expr(e) => TStmt::Expr(self.expr(e)),
            TStmt::If(c, a, b) => TStmt::If(self.expr(c), self.stmts(a), self.stmts(b)),
            TStmt::Loop { id, cond, test_after, update, body } => {
                let cond = cond.map(|c| self.expr(c));
                let update = update.map(|u| self.expr(u));
                self.loops.push(id);
                let body = self.stmts(body);
                self.loops.pop();
                TStmt::Loop { id, cond, test_after, update, body }
            }
            TStmt::ForOf { id, var, arr, body } => {
                let arr = self.expr(arr);
                self.loops.push(id);
                let body = self.stmts(body);
                self.loops.pop();
                TStmt::ForOf { id, var, arr, body }
            }
            TStmt::Return(e) => TStmt::Return(e.map(|e| self.expr(e))),
            TStmt::Switch { disc, eq, cases } => TStmt::Switch {
                disc: self.expr(disc),
                eq,
                cases: cases.into_iter().map(|(t, b)| (t.map(|t| self.expr(t)), self.stmts(b))).collect(),
            },
            TStmt::Throw(e) => TStmt::Throw(self.expr(e)),
            TStmt::Block(b) => TStmt::Block(self.stmts(b)),
            s @ (TStmt::Break | TStmt::Continue) => s,
        }
    }

    fn bx(&mut self, e: Box<TExpr>) -> Box<TExpr> {
        Box::new(self.expr(*e))
    }

    /// Hot reload (SPEC.md §13): `v` is a `let` at the top level of a
    /// module (not inside a component, a loop or a closure — `module`
    /// is only set for those), its initializer is a plain `signal(...)`
    /// and its value type has a shape `sig_register` understands.
    fn is_registrable_module_signal(&self, v: VarId) -> bool {
        matches!(self.prog.funcs[self.func as usize].kind, FuncKind::ModuleInit(_))
            && self.prog.vars[v as usize].module.is_some()
            && matches!(&self.prog.vars[v as usize].ty, Type::Signal(inner) if shape_code(inner).is_some())
    }

    fn expr(&mut self, e: TExpr) -> TExpr {
        let TExpr { kind, ty, span } = e;
        let kind = match kind {
            TExprKind::SignalNew(init) => {
                let init = self.expr(*init);
                let f = by_repr(&init.ty, "sig_new_f64", "sig_new_i32", "sig_new_ref");
                TExprKind::Rt(f, vec![init])
            }
            TExprKind::SignalGet(s) => {
                let f = by_repr(&ty, "sig_get_f64", "sig_get_i32", "sig_get_i32");
                TExprKind::Rt(f, vec![self.expr(*s)])
            }
            TExprKind::SignalPeek(s) => {
                let f = by_repr(&ty, "sig_peek_f64", "sig_peek_i32", "sig_peek_i32");
                TExprKind::Rt(f, vec![self.expr(*s)])
            }
            TExprKind::SignalSet(s, v) => {
                let v = self.expr(*v);
                let f = by_repr(&v.ty, "sig_set_f64", "sig_set_i32", "sig_set_ref");
                TExprKind::Rt(f, vec![self.expr(*s), v])
            }
            TExprKind::ComputedNew(f) => {
                let f = self.expr(*f);
                let thunk = thunk_of(&f);
                TExprKind::Rt("comp_new", vec![thunk, f])
            }
            TExprKind::ComputedGet(c) => {
                let f = by_repr(&ty, "comp_get_f64", "comp_get_i32", "comp_get_i32");
                TExprKind::Rt(f, vec![self.expr(*c)])
            }
            TExprKind::EffectNew(f) => {
                let f = self.expr(*f);
                let thunk = thunk_of(&f);
                TExprKind::Rt("effect", vec![thunk, f])
            }
            TExprKind::TimerNew(ms, repeat, f) => {
                let ms = self.expr(*ms);
                let f = self.expr(*f);
                let thunk = thunk_of(&f);
                TExprKind::Rt("set_timer", vec![thunk, f, ms, i32c(repeat as i32)])
            }
            TExprKind::Navigate(name) => {
                let idx = self.prog.screens.iter().position(|s| s.name == name).unwrap_or(0);
                TExprKind::Rt("navigate", vec![i32c(idx as i32)])
            }
            TExprKind::NavigatePush(name) => {
                let idx = self.prog.screens.iter().position(|s| s.name == name).unwrap_or(0);
                TExprKind::Rt("navigate_push", vec![i32c(idx as i32)])
            }
            TExprKind::NavigateBack => TExprKind::Rt("navigate_back", Vec::new()),
            TExprKind::Jsx(j) => return self.jsx(*j, span),
            // Structural recursion for the rest.
            TExprKind::Assign(p, v) => {
                let p = match p {
                    Place::Var(v) => Place::Var(v),
                    Place::Field(o, s, i) => Place::Field(self.bx(o), s, i),
                    Place::Index(a, i) => Place::Index(self.bx(a), self.bx(i)),
                };
                TExprKind::Assign(p, self.bx(v))
            }
            TExprKind::Field(o, s, i) => TExprKind::Field(self.bx(o), s, i),
            TExprKind::Index(a, i) => TExprKind::Index(self.bx(a), self.bx(i)),
            TExprKind::Call(f, args) => TExprKind::Call(f, args.into_iter().map(|a| self.expr(a)).collect()),
            TExprKind::CallClosure(c, args) => {
                TExprKind::CallClosure(self.bx(c), args.into_iter().map(|a| self.expr(a)).collect())
            }
            TExprKind::Num2(op, a, b) => TExprKind::Num2(op, self.bx(a), self.bx(b)),
            TExprKind::Int2(op, a, b) => TExprKind::Int2(op, self.bx(a), self.bx(b)),
            TExprKind::Neg(a) => TExprKind::Neg(self.bx(a)),
            TExprKind::Not(a) => TExprKind::Not(self.bx(a)),
            TExprKind::Cmp(op, k, a, b) => TExprKind::Cmp(op, k, self.bx(a), self.bx(b)),
            TExprKind::StrCmp(op, a, b) => TExprKind::StrCmp(op, self.bx(a), self.bx(b)),
            TExprKind::Concat(a, b) => TExprKind::Concat(self.bx(a), self.bx(b)),
            TExprKind::And(a, b) => TExprKind::And(self.bx(a), self.bx(b)),
            TExprKind::Or(a, b) => TExprKind::Or(self.bx(a), self.bx(b)),
            TExprKind::Cond(a, b, c) => TExprKind::Cond(self.bx(a), self.bx(b), self.bx(c)),
            TExprKind::IsNull(a) => TExprKind::IsNull(self.bx(a)),
            TExprKind::UnionTag(a) => TExprKind::UnionTag(self.bx(a)),
            TExprKind::UnionIs(a, idxs) => TExprKind::UnionIs(self.bx(a), idxs),
            TExprKind::Coerce(c, a) => TExprKind::Coerce(c, self.bx(a)),
            TExprKind::Block(stmts, v) => TExprKind::Block(self.stmts(stmts), self.bx(v)),
            TExprKind::ArrayLit(items) => TExprKind::ArrayLit(items.into_iter().map(|(s, a)| (s, self.expr(a))).collect()),
            TExprKind::StructLit(s, vals) => TExprKind::StructLit(s, vals.into_iter().map(|a| self.expr(a)).collect()),
            TExprKind::ArrayHof { kind, arr, f, arity } => TExprKind::ArrayHof { kind, arr: self.bx(arr), f: self.bx(f), arity },
            TExprKind::ArraySearch { index, eq, arr, value } => {
                TExprKind::ArraySearch { index, eq, arr: self.bx(arr), value: self.bx(value) }
            }
            TExprKind::Rt(n, args) => TExprKind::Rt(n, args.into_iter().map(|a| self.expr(a)).collect()),
            TExprKind::MathOp(op, args) => TExprKind::MathOp(op, args.into_iter().map(|a| self.expr(a)).collect()),
            k @ (TExprKind::Num(_)
            | TExprKind::Bool(_)
            | TExprKind::Str(_)
            | TExprKind::Null
            | TExprKind::Var(_)
            | TExprKind::Closure(_)
            | TExprKind::ThunkOf(_)) => k,
        };
        TExpr { kind, ty, span }
    }

    // -- Helpers ----------------------------------------------------------

    fn new_var(&mut self, name: &str, ty: Type) -> VarId {
        self.prog.new_var(VarInfo {
            name: name.to_owned(),
            ty,
            owner: self.func,
            mutable: false,
            module: None,
            in_loop: self.loops.last().copied(),
            captured: false,
        })
    }

    /// A closure `() => body`, owned by the current function's scope. The
    /// temporaries declared inside `body` move into the new function.
    fn synthetic(&mut self, name: &str, body: Vec<TStmt>, ret: Type, span: Span) -> TExpr {
        let fid = self.prog.new_func(FuncDef {
            name: name.to_owned(),
            kind: FuncKind::Closure,
            params: Vec::new(),
            ret: ret.clone(),
            body: Vec::new(),
            span,
        });
        let mut declared = Vec::new();
        for s in &body {
            collect_lets(s, &mut declared);
        }
        for v in declared {
            let info = &mut self.prog.vars[v as usize];
            if info.owner == self.func {
                info.owner = fid;
                info.in_loop = None;
            }
        }
        self.prog.funcs[fid as usize].body = body;
        let ft = FuncType { params: Vec::new(), required: 0, ret };
        TExpr::new(TExprKind::Closure(fid), Type::Func(Rc::new(ft)), span)
    }

    /// `() => value`
    fn value_closure(&mut self, name: &str, value: TExpr) -> TExpr {
        let span = value.span;
        let ty = value.ty.clone();
        self.synthetic(name, vec![TStmt::Return(Some(value))], ty, span)
    }

    /// Runs `stmt` now if `value` is static, or in an effect if it is
    /// dynamic.
    fn maybe_effect(&mut self, dynamic: bool, stmt: TStmt, out: &mut Vec<TStmt>, span: Span) {
        if !dynamic {
            out.push(stmt);
            return;
        }
        let f = self.synthetic("<effect>", vec![stmt], Type::Void, span);
        let thunk = thunk_of(&f);
        out.push(rt_stmt("effect", vec![thunk, f]));
    }

    // -- JSX ----------------------------------------------------------------

    fn jsx(&mut self, j: TJsx, span: Span) -> TExpr {
        match j {
            TJsx::Component { func, props, span } => {
                let args = props.map(|p| vec![self.expr(p)]).unwrap_or_default();
                TExpr::new(TExprKind::Call(func, args), Type::Element, span)
            }
            TJsx::Control { kind, props, children, .. } => {
                let mut out = Vec::new();
                let n = self.new_var("$node", Type::Element);
                out.push(TStmt::Let(n, Some(rt("node", vec![i32c(kind as i32)], Type::Element))));
                let node = || TExpr::new(TExprKind::Var(n), Type::Element, span);
                let (mut items, mut key, mut row, mut empty) = (None, None, None, None);
                for p in props {
                    let value = self.expr(p.value);
                    let dynamic = is_dynamic(&value);
                    match p.target {
                        PropTarget::Str(id) => {
                            let s = rt_stmt("prop_str", vec![node(), i32c(id as i32), value]);
                            self.maybe_effect(dynamic, s, &mut out, span);
                        }
                        PropTarget::Num(id) => {
                            let s = rt_stmt("prop_f64", vec![node(), i32c(id as i32), value]);
                            self.maybe_effect(dynamic, s, &mut out, span);
                        }
                        PropTarget::Int(id) => {
                            let v = TExpr::new(TExprKind::Coerce(Coercion::NumToI32, Box::new(value)), Type::Bool, span);
                            let s = rt_stmt("prop_int", vec![node(), i32c(id as i32), v]);
                            self.maybe_effect(dynamic, s, &mut out, span);
                        }
                        PropTarget::Bool(id) => {
                            let s = rt_stmt("prop_bool", vec![node(), i32c(id as i32), value]);
                            self.maybe_effect(dynamic, s, &mut out, span);
                        }
                        PropTarget::Enum(id, table) => {
                            let v = self.enum_value(value, &table);
                            let s = rt_stmt("prop_enum", vec![node(), i32c(id as i32), v]);
                            self.maybe_effect(dynamic, s, &mut out, span);
                        }
                        PropTarget::Event(id) => {
                            let thunk = thunk_of(&value);
                            out.push(rt_stmt("listen", vec![node(), i32c(id as i32), thunk, value]));
                        }
                        PropTarget::Bind { kind } => {
                            let k = match kind {
                                BindValKind::Str => 0,
                                BindValKind::Bool => 1,
                                BindValKind::Num => 2,
                            };
                            out.push(rt_stmt("bind", vec![node(), value, i32c(k)]));
                        }
                        PropTarget::ListItems => items = Some(value),
                        PropTarget::ListKey => key = Some(value),
                        PropTarget::ListRow => row = Some(value),
                        PropTarget::ListEmpty => empty = Some(value),
                    }
                }
                if let (Some(items), Some(key), Some(row)) = (items, key, row) {
                    let items = self.value_closure("<items>", items);
                    let (it, kt, rt_) = (thunk_of(&items), thunk_of(&key), thunk_of(&row));
                    let (et, ee) = match empty {
                        Some(e) => {
                            let c = self.value_closure("<empty>", e);
                            (thunk_of(&c), c)
                        }
                        None => (i32c(0), TExpr::new(TExprKind::Null, Type::Null, span)),
                    };
                    out.push(rt_stmt("list", vec![node(), it, items, kt, key, rt_, row, et, ee]));
                }
                match children {
                    TChildren::None => {}
                    TChildren::Text(parts) => {
                        let mut text: Option<TExpr> = None;
                        for p in parts {
                            let p = self.expr(p);
                            text = Some(match text {
                                None => p,
                                Some(prev) => TExpr::new(TExprKind::Concat(Box::new(prev), Box::new(p)), Type::String, span),
                            });
                        }
                        if let Some(t) = text {
                            let dynamic = is_dynamic(&t);
                            self.maybe_effect(dynamic, rt_stmt("text", vec![node(), t]), &mut out, span);
                        }
                    }
                    TChildren::Nodes(kids) => {
                        for k in kids {
                            match k {
                                TChild::Element(j) => {
                                    let c = self.jsx(j, span);
                                    out.push(rt_stmt("append", vec![node(), c]));
                                }
                                TChild::Expr(e) => {
                                    let e = self.expr(e);
                                    let c = self.value_closure("<region>", e);
                                    let thunk = thunk_of(&c);
                                    out.push(rt_stmt("region", vec![node(), thunk, c]));
                                }
                            }
                        }
                    }
                }
                TExpr::new(TExprKind::Block(out, Box::new(node())), Type::Element, span)
            }
        }
    }

    /// Maps a string literal union value to its enum id.
    fn enum_value(&mut self, value: TExpr, table: &[(String, u16)]) -> TExpr {
        let span = value.span;
        let inner = match &value.kind {
            TExprKind::Coerce(Coercion::Retag, inner) => inner.as_ref(),
            _ => &value,
        };
        if let TExprKind::Str(s) = &inner.kind {
            let id = table.iter().find(|(n, _)| n == s).map(|(_, v)| *v).unwrap_or(0);
            return i32c(id as i32);
        }
        // tmp = value; tmp === "a" ? 0 : tmp === "b" ? 1 : ...
        let tmp = self.new_var("$enum", Type::String);
        let read = TExpr::new(TExprKind::Var(tmp), Type::String, span);
        let mut acc = i32c(table.first().map(|(_, v)| *v as i32).unwrap_or(0));
        for (name, id) in table.iter().rev() {
            let lit = TExpr::new(TExprKind::Str(name.clone()), Type::String, span);
            let test = TExpr::new(TExprKind::Cmp(CmpOp::Eq, EqKind::Str, Box::new(read.clone()), Box::new(lit)), Type::Bool, span);
            acc = TExpr::new(TExprKind::Cond(Box::new(test), Box::new(i32c(*id as i32)), Box::new(acc)), Type::Bool, span);
        }
        TExpr::new(TExprKind::Block(vec![TStmt::Let(tmp, Some(value))], Box::new(acc)), Type::Bool, span)
    }
}

/// The thunk signature of a closure value, from its type.
pub fn thunk_sig(ft: &FuncType) -> ThunkSig {
    ThunkSig { params: ft.params.iter().map(|t| t.repr()).collect(), ret: ft.ret.repr() }
}

fn thunk_of(f: &TExpr) -> TExpr {
    let sig = match &f.ty {
        Type::Func(ft) => thunk_sig(ft),
        _ => ThunkSig { params: Vec::new(), ret: Repr::Void },
    };
    TExpr::new(TExprKind::ThunkOf(sig), Type::Bool, f.span)
}

fn collect_lets(s: &TStmt, out: &mut Vec<VarId>) {
    let from_expr = |e: &TExpr, out: &mut Vec<VarId>| {
        visit_expr(e, &mut |x| {
            if let TExprKind::Block(stmts, _) = &x.kind {
                for s in stmts {
                    if let TStmt::Let(v, _) = s {
                        out.push(*v);
                    }
                }
            }
        });
    };
    match s {
        TStmt::Let(v, e) => {
            out.push(*v);
            if let Some(e) = e {
                from_expr(e, out);
            }
        }
        TStmt::Expr(e) | TStmt::Return(Some(e)) | TStmt::Throw(e) => from_expr(e, out),
        TStmt::If(c, a, b) => {
            from_expr(c, out);
            for x in a.iter().chain(b) {
                collect_lets(x, out);
            }
        }
        TStmt::Block(b) => {
            for x in b {
                collect_lets(x, out);
            }
        }
        _ => {}
    }
}
