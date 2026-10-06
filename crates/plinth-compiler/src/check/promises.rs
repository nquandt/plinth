//! The `Promise` API (SPEC.md §4.5): `then`, `catch`, `finally`,
//! `new Promise`, `Promise.all`, `Promise.resolve` and `Promise.reject`.
//!
//! All of it is generated code on top of the promise helpers in
//! `asyncfn.rs`: a callback of `then` becomes a waiter closure that the
//! shared `then(p, k)` registers, and the waiter settles the new promise.
//! A callback that returns a promise is adopted: the new promise settles
//! when that promise settles. An exception in a callback rejects the new
//! promise.

use super::Checker;
use super::asyncfn::{ERROR, PromiseInfo, STATE, VALUE, bx, int, is, set_var, value_ty, void_stmt};
use crate::ast::{self, Expr, ExprKind};
use crate::diag::{Span, code};
use crate::tir::*;
use crate::types::{FuncType, Type};
use std::rc::Rc;

fn var(v: VarId, ty: &Type, span: Span) -> TExpr {
    TExpr::new(TExprKind::Var(v), ty.clone(), span)
}

fn void_fn(params: Vec<Type>) -> Type {
    let n = params.len();
    Type::Func(Rc::new(FuncType { params, required: n, ret: Type::Void }))
}

fn ret_of(f: &TExpr) -> Type {
    match &f.ty {
        Type::Func(ft) => ft.ret.clone(),
        _ => Type::Error,
    }
}

impl Checker<'_> {
    fn err_expr(&self, span: Span) -> TExpr {
        TExpr::new(TExprKind::Null, Type::Error, span)
    }

    /// A closure with these parameter types; `build` gets its id and its
    /// parameter variables and returns its body.
    fn gen_closure(&mut self, name: &str, params: Vec<Type>, span: Span, build: impl FnOnce(&mut Self, FuncId, &[VarId]) -> Vec<TStmt>) -> TExpr {
        let fid = self.prog.new_func(FuncDef { name: name.to_owned(), kind: FuncKind::Closure, params: Vec::new(), ret: Type::Void, body: Vec::new(), span });
        let vars: Vec<VarId> = params.iter().map(|t| self.var_in(fid, "$x", t.clone(), None)).collect();
        self.prog.funcs[fid as usize].params = vars.clone();
        let body = build(self, fid, &vars);
        self.prog.funcs[fid as usize].body = body;
        TExpr::new(TExprKind::Closure(fid), void_fn(params), span)
    }

    /// `then(q, k)`: run the waiter `k` when `q` settles.
    fn then_stmt(&self, q: TExpr, k: TExpr, span: Span) -> TStmt {
        let (then, _) = self.promise_then_reject();
        void_stmt(TExprKind::Call(then, vec![self.as_base(q), k]), span)
    }

    /// `reject(r, e)`.
    fn reject_stmt(&self, r: TExpr, e: TExpr, span: Span) -> TStmt {
        let (_, reject) = self.promise_then_reject();
        void_stmt(TExprKind::Call(reject, vec![self.as_base(r), e]), span)
    }

    /// `resolve(r, x)` for a value `x` of the type of `r` (any value for a
    /// `Promise<void>`).
    fn resolve_stmts(&mut self, r: TExpr, info: &PromiseInfo, x: TExpr, span: Span) -> Vec<TStmt> {
        if info.ty == Type::Void {
            let mut out = Vec::new();
            if !matches!(x.kind, TExprKind::Bool(_) | TExprKind::Null) {
                out.push(TStmt::Expr(x));
            }
            out.push(void_stmt(TExprKind::Call(info.resolve, vec![r, TExpr::new(TExprKind::Bool(false), Type::Bool, span)]), span));
            return out;
        }
        let x = self.coerce(x, &info.ty);
        vec![void_stmt(TExprKind::Call(info.resolve, vec![r, x]), span)]
    }

    /// The error of a rejected promise `q`.
    fn error_of(&self, q: &TExpr, sid: crate::types::StructId) -> TExpr {
        let err_ty = self.error_type();
        let field = self.pfield(q, sid, ERROR, err_ty.clone().nullable());
        TExpr::new(TExprKind::Coerce(Coercion::Retag, bx(field)), err_ty, q.span)
    }

    /// The value of a fulfilled promise `q` (a `boolean` for `Promise<void>`).
    fn value_of(&self, q: &TExpr, info: &PromiseInfo) -> TExpr {
        self.pfield(q, info.sid, VALUE, value_ty(&info.ty))
    }

    /// `q.#state === n`.
    fn state_is(&self, q: &TExpr, n: i32) -> TExpr {
        is(self.pfield(q, self.promise_of(&q.ty).map(|i| i.sid).unwrap_or(0), STATE, Type::Int), n, q.span)
    }

    /// Settles `r` as the settled `q` is: the same value or error.
    fn copy_settled(&mut self, q: &TExpr, qinfo: &PromiseInfo, r: &TExpr, rinfo: &PromiseInfo, span: Span) -> Vec<TStmt> {
        let ok = self.value_of(q, qinfo);
        let ok = if rinfo.ty == Type::Void { TExpr::new(TExprKind::Bool(false), Type::Bool, span) } else { ok };
        let ok = self.resolve_stmts(r.clone(), rinfo, ok, span);
        let err = self.reject_stmt(r.clone(), self.error_of(q, qinfo.sid), span);
        vec![TStmt::If(self.state_is(q, 1), ok, vec![err])]
    }

    /// Settles `r` with the result `x` of a callback: its value, or, when
    /// `x` is a promise, what that promise settles to.
    fn settle_with(&mut self, owner: FuncId, r: &TExpr, rinfo: &PromiseInfo, x: TExpr, span: Span) -> Vec<TStmt> {
        let Some(xinfo) = self.promise_of(&x.ty).cloned() else {
            return self.resolve_stmts(r.clone(), rinfo, x, span);
        };
        let xv = self.var_in(owner, "$x", x.ty.clone(), None);
        let xr = var(xv, &x.ty, span);
        let r2 = r.clone();
        let rinfo2 = rinfo.clone();
        let xr2 = xr.clone();
        let k = self.gen_closure("<adopt>", Vec::new(), span, |c, _, _| c.copy_settled(&xr2, &xinfo, &r2, &rinfo2, span));
        vec![TStmt::Let(xv, Some(x)), self.then_stmt(xr, k, span)]
    }

    /// `try { body } catch (e) { reject(r, e) }` in the closure `owner`.
    fn reject_on_throw(&mut self, owner: FuncId, r: &TExpr, body: Vec<TStmt>, span: Span) -> Vec<TStmt> {
        let err_ty = self.error_type();
        let ev = self.var_in(owner, "$e", err_ty.clone(), None);
        let rej = self.reject_stmt(r.clone(), var(ev, &err_ty, span), span);
        vec![TStmt::Try { body, catch: Some((ev, vec![rej])), finally: None }]
    }

    /// A call of the callback in `f` (a variable) with the first `arity`
    /// of `args`.
    fn call_cb(&self, f: &TExpr, arity: usize, mut args: Vec<TExpr>, span: Span) -> TExpr {
        args.truncate(arity);
        TExpr::new(TExprKind::CallClosure(bx(f.clone()), args), ret_of(f), span)
    }

    /// `t` without one level of `Promise`.
    fn awaited(&self, t: &Type) -> Type {
        self.promise_of(t).map(|i| i.ty.clone()).unwrap_or_else(|| t.clone())
    }

    /// Checks that a callback result of type `got` can settle a
    /// `Promise<want>`.
    fn check_settles(&mut self, got: &Type, want: &Type, what: &str, span: Span) -> bool {
        let got = self.awaited(got);
        if *want == Type::Void || got.is_error() || want.is_error() || self.conversion(&got, want).is_some() {
            return true;
        }
        let msg = format!("{what} must return `{}`, not `{}`", self.show(want), self.show(&got));
        self.err(code::TYPE_MISMATCH, span, msg);
        false
    }

    /// `p.then(f, g?)`, `p.catch(g)` and `p.finally(h)`.
    pub(super) fn promise_method(&mut self, o: TExpr, info: PromiseInfo, prop: &str, prop_span: Span, args: &[Expr], span: Span) -> TExpr {
        let t = info.ty.clone();
        let err_ty = self.error_type();
        let want = match prop {
            "then" => 1..=2,
            "catch" | "finally" => 1..=1,
            _ => {
                let msg = format!("`{}` has no method `{prop}`", self.show(&o.ty));
                self.err_help(code::NO_PROPERTY, prop_span, msg, "use `await`, `then`, `catch` or `finally`");
                return self.err_expr(span);
            }
        };
        if !want.contains(&args.len()) {
            let msg = if prop == "then" { "`then` takes one or two functions".to_string() } else { format!("`{prop}` takes one function") };
            self.err(code::ARG_COUNT, span, msg);
            for a in args {
                self.expr(a, None);
            }
            return self.err_expr(span);
        }
        let value_params = if t == Type::Void { Vec::new() } else { vec![t.clone()] };
        let pty = o.ty.clone();
        let qv = self.temp(pty.clone());
        let q = var(qv, &pty, span);
        let mut pre = vec![TStmt::Let(qv, Some(o))];
        // The callbacks, each in a variable.
        let cb = |c: &mut Self, e: &Expr, params: &[Type], pre: &mut Vec<TStmt>| {
            let (f, arity) = c.callback(e, params, None);
            let fv = c.temp(f.ty.clone());
            let fr = var(fv, &f.ty, e.span);
            pre.push(TStmt::Let(fv, Some(f)));
            (fr, arity)
        };
        let (rty, waiter): (Type, Box<dyn FnOnce(&mut Self, FuncId, &TExpr, &PromiseInfo) -> Vec<TStmt>>) = match prop {
            "then" => {
                let (f, fa) = cb(self, &args[0], &value_params, &mut pre);
                let u = self.awaited(&ret_of(&f));
                let mut g = None;
                if let Some(a) = args.get(1) {
                    let (gf, ga) = cb(self, a, std::slice::from_ref(&err_ty), &mut pre);
                    if !self.check_settles(&ret_of(&gf), &u, "the `onRejected` function", a.span) {
                        return self.err_expr(span);
                    }
                    g = Some((gf, ga));
                }
                let (q2, info2) = (q.clone(), info.clone());
                (
                    u,
                    Box::new(move |c: &mut Self, kf, r, rinfo| {
                        let v = c.value_of(&q2, &info2);
                        let x = c.call_cb(&f, fa, vec![v], span);
                        let ok = c.settle_with(kf, r, rinfo, x, span);
                        let err = c.error_of(&q2, info2.sid);
                        let bad = match g {
                            Some((g, ga)) => {
                                let x = c.call_cb(&g, ga, vec![err], span);
                                c.settle_with(kf, r, rinfo, x, span)
                            }
                            None => vec![c.reject_stmt(r.clone(), err, span)],
                        };
                        vec![TStmt::If(c.state_is(&q2, 1), ok, bad)]
                    }),
                )
            }
            "catch" => {
                let (g, ga) = cb(self, &args[0], std::slice::from_ref(&err_ty), &mut pre);
                if !self.check_settles(&ret_of(&g), &t, "the `catch` function", args[0].span) {
                    return self.err_expr(span);
                }
                let (q2, info2) = (q.clone(), info.clone());
                (
                    t.clone(),
                    Box::new(move |c: &mut Self, kf, r, rinfo| {
                        let v = c.value_of(&q2, &info2);
                        let ok = c.resolve_stmts(r.clone(), rinfo, v, span);
                        let err = c.error_of(&q2, info2.sid);
                        let x = c.call_cb(&g, ga, vec![err], span);
                        let bad = c.settle_with(kf, r, rinfo, x, span);
                        vec![TStmt::If(c.state_is(&q2, 1), ok, bad)]
                    }),
                )
            }
            _ => {
                let (h, ha) = cb(self, &args[0], &[], &mut pre);
                let (q2, info2) = (q.clone(), info.clone());
                (
                    t.clone(),
                    Box::new(move |c: &mut Self, kf, r, rinfo| {
                        let x = c.call_cb(&h, ha, Vec::new(), span);
                        match c.promise_of(&x.ty).cloned() {
                            // Wait for the promise that `h` returns; its
                            // rejection replaces the result.
                            Some(xinfo) => {
                                let xv = c.var_in(kf, "$x", x.ty.clone(), None);
                                let xr = var(xv, &x.ty, span);
                                let (xr2, r2, rinfo2) = (xr.clone(), r.clone(), rinfo.clone());
                                let k = c.gen_closure("<finally wait>", Vec::new(), span, |c, _, _| {
                                    let bad = c.reject_stmt(r2.clone(), c.error_of(&xr2, xinfo.sid), span);
                                    let copy = c.copy_settled(&q2, &info2, &r2, &rinfo2, span);
                                    vec![TStmt::If(c.state_is(&xr2, 2), vec![bad], copy)]
                                });
                                vec![TStmt::Let(xv, Some(x)), c.then_stmt(xr, k, span)]
                            }
                            None => {
                                let mut out = vec![TStmt::Expr(x)];
                                out.extend(c.copy_settled(&q2, &info2, r, rinfo, span));
                                out
                            }
                        }
                    }),
                )
            }
        };
        if rty.is_error() {
            return self.err_expr(span);
        }
        let rinfo = self.promise_info(&rty);
        let rpty = Type::Struct(rinfo.sid);
        let rv = self.temp(rpty.clone());
        let r = var(rv, &rpty, span);
        pre.push(TStmt::Let(rv, Some(self.new_promise(&rinfo, span))));
        let (r2, rinfo2) = (r.clone(), rinfo.clone());
        let k = self.gen_closure("<then>", Vec::new(), span, |c, kf, _| {
            let body = waiter(c, kf, &r2, &rinfo2);
            c.reject_on_throw(kf, &r2, body, span)
        });
        pre.push(self.then_stmt(q, k, span));
        TExpr::new(TExprKind::Block(pre, bx(r)), rpty, span)
    }

    /// `Promise.all`, `Promise.resolve` and `Promise.reject`.
    pub(super) fn promise_static(&mut self, prop: &str, prop_span: Span, type_args: &[ast::TypeAnn], args: &[Expr], span: Span, expected: Option<&Type>) -> TExpr {
        let expected_inner = expected.and_then(|e| self.promise_of(e)).map(|i| i.ty.clone());
        match prop {
            "resolve" => {
                if args.len() > 1 {
                    self.err(code::ARG_COUNT, span, "`Promise.resolve` takes zero or one argument");
                }
                let Some(a) = args.first() else {
                    let info = self.promise_info(&Type::Void);
                    return self.settled_promise(&info, TExpr::new(TExprKind::Bool(false), Type::Bool, span), span);
                };
                let v = match &expected_inner {
                    Some(t) if *t != Type::Void => self.expr_with(a, t),
                    _ => self.expr(a, None),
                };
                if v.ty.is_error() || self.promise_of(&v.ty).is_some() {
                    return v;
                }
                let t = match &expected_inner {
                    Some(t) if *t != Type::Void && self.conversion(&v.ty, t).is_some() => t.clone(),
                    _ => self.widen(v.ty.clone()),
                };
                let v = self.coerce(v, &t);
                let info = self.promise_info(&t);
                self.settled_promise(&info, v, span)
            }
            "reject" => {
                if args.len() != 1 {
                    self.err(code::ARG_COUNT, span, "`Promise.reject` takes one argument");
                    return self.err_expr(span);
                }
                let t = match (type_args, expected_inner) {
                    ([t], _) => self.resolve_type(t),
                    ([], Some(t)) => t,
                    _ => Type::Void,
                };
                let e = self.error_value(&args[0]);
                if t.is_error() || e.ty.is_error() {
                    return self.err_expr(span);
                }
                let info = self.promise_info(&t);
                let pty = Type::Struct(info.sid);
                let pv = self.temp(pty.clone());
                let p = var(pv, &pty, span);
                let stmts = vec![TStmt::Let(pv, Some(self.new_promise(&info, span))), self.reject_stmt(p.clone(), e, span)];
                TExpr::new(TExprKind::Block(stmts, bx(p)), pty, span)
            }
            "all" => {
                if args.len() != 1 {
                    self.err(code::ARG_COUNT, span, "`Promise.all` takes one array of promises");
                    return self.err_expr(span);
                }
                self.promise_all(&args[0], span)
            }
            _ => {
                let msg = format!("`Promise.{prop}` is not supported");
                self.err_help(code::NO_PROPERTY, prop_span, msg, "use `Promise.all`, `Promise.resolve` or `Promise.reject`");
                self.err_expr(span)
            }
        }
    }

    /// A `Promise` that is already fulfilled with `v`.
    fn settled_promise(&self, info: &PromiseInfo, v: TExpr, span: Span) -> TExpr {
        let TExpr { kind: TExprKind::StructLit(sid, mut fields), ty, .. } = self.new_promise(info, span) else { unreachable!() };
        fields[STATE as usize] = int(1, span);
        fields[VALUE as usize] = v;
        TExpr::new(TExprKind::StructLit(sid, fields), ty, span)
    }

    /// The thrown value of `reject(e)`: an `Error`, or a string as
    /// `new Error(s)`, as `throw` takes it.
    pub(super) fn error_value(&mut self, e: &Expr) -> TExpr {
        let error_ty = self.error_type();
        let te = self.expr(e, Some(&error_ty));
        if te.ty.is_stringish() {
            self.new_error(te)
        } else if self.is_error_class(&te.ty) {
            self.coerce(te, &error_ty)
        } else {
            if !te.ty.is_error() {
                let msg = format!("expected an `Error` or a string, not `{}`", self.show(&te.ty));
                self.err_help(code::TYPE_MISMATCH, e.span, msg, "write `new Error(\"message\")`");
            }
            TExpr { ty: Type::Error, ..te }
        }
    }

    /// `Promise.all(a)`: an array of `Promise<T>` gives a `Promise<T[]>`
    /// (a `Promise<void>` for `Promise<void>[]`); an array literal of
    /// promises of different types gives a promise of a tuple.
    fn promise_all(&mut self, a: &Expr, span: Span) -> TExpr {
        if let ExprKind::Array(elems) = &a.kind
            && !elems.is_empty()
            && elems.iter().all(|(spread, _)| !*spread)
        {
            let items: Vec<TExpr> = elems.iter().map(|(_, e)| self.expr(e, None)).collect();
            if items.iter().any(|i| i.ty.is_error()) {
                return self.err_expr(span);
            }
            for (i, (_, e)) in items.iter().zip(elems) {
                if self.promise_of(&i.ty).is_none() {
                    let msg = format!("`Promise.all` needs promises, not `{}`", self.show(&i.ty));
                    self.err(code::TYPE_MISMATCH, e.span, msg);
                    return self.err_expr(span);
                }
            }
            if items.iter().all(|i| i.ty == items[0].ty) {
                let aty = Type::Array(Box::new(items[0].ty.clone()));
                let lit = TExpr::new(TExprKind::ArrayLit(items.into_iter().map(|i| (false, i)).collect()), aty, a.span);
                return self.promise_all_array(lit, span);
            }
            return self.promise_all_tuple(items, span);
        }
        let arr = self.expr(a, None);
        match &arr.ty {
            Type::Array(e) if self.promise_of(e).is_some() => self.promise_all_array(arr, span),
            Type::Error => self.err_expr(span),
            other => {
                let msg = format!("`Promise.all` needs an array of promises, not `{}`", self.show(other));
                self.err(code::TYPE_MISMATCH, a.span, msg);
                self.err_expr(span)
            }
        }
    }

    fn promise_all_array(&mut self, arr: TExpr, span: Span) -> TExpr {
        let Type::Array(e) = &arr.ty else { unreachable!() };
        let qinfo = self.promise_of(e).cloned().expect("checked by the caller");
        let fid = match self.promise_alls.iter().find(|(t, _)| *t == qinfo.ty) {
            Some((_, f)) => *f,
            None => {
                let f = self.make_promise_all(&qinfo);
                self.promise_alls.push((qinfo.ty.clone(), f));
                f
            }
        };
        let ret = self.prog.funcs[fid as usize].ret.clone();
        TExpr::new(TExprKind::Call(fid, vec![arr]), ret, span)
    }

    /// `all(ps: Promise<T>[]): Promise<T[]>`: copies `ps`, waits for each
    /// promise, rejects with the first rejection, and resolves with the
    /// values in order when the last one fulfills.
    fn make_promise_all(&mut self, qinfo: &PromiseInfo) -> FuncId {
        let span = Span::default();
        let qty = Type::Struct(qinfo.sid);
        let aty = Type::Array(Box::new(qty.clone()));
        let void = qinfo.ty == Type::Void;
        let out_ty = if void { Type::Void } else { Type::Array(Box::new(qinfo.ty.clone())) };
        let rinfo = self.promise_info(&out_ty);
        let rty = Type::Struct(rinfo.sid);
        let (fid, ps) = self.helper_fn(&format!("Promise.all<{}>", self.show(&qinfo.ty)), &[("ps", aty.clone())], rty.clone());
        let av = self.var_in(fid, "$a", aty.clone(), None);
        let a = var(av, &aty, span);
        let rv = self.var_in(fid, "$r", rty.clone(), None);
        let r = var(rv, &rty, span);
        let lv = self.var_in(fid, "$left", Type::Int, None);
        let left = var(lv, &Type::Int, span);
        let id = self.prog.new_loop();
        let qv = self.var_in(fid, "$q", qty.clone(), Some(id));
        let q = var(qv, &qty, span);

        // The waiter for one element.
        let (q2, r2, rinfo2, a2, left2, qinfo2) = (q.clone(), r.clone(), rinfo.clone(), a.clone(), left.clone(), qinfo.clone());
        let k = self.gen_closure("<all>", Vec::new(), span, |c, kf, _| {
            let bad = c.reject_stmt(r2.clone(), c.error_of(&q2, qinfo2.sid), span);
            let dec = TExpr::new(TExprKind::Int2(IntOp::Sub, bx(left2.clone()), bx(int(1, span))), Type::Int, span);
            let mut done = Vec::new();
            if void {
                done.extend(c.resolve_stmts(r2.clone(), &rinfo2, TExpr::new(TExprKind::Bool(false), Type::Bool, span), span));
            } else {
                let oty = out_ty.clone();
                let ov = c.var_in(kf, "$out", oty.clone(), None);
                let o = var(ov, &oty, span);
                let id2 = c.prog.new_loop();
                let xv = c.var_in(kf, "$p", qty.clone(), Some(id2));
                let x = var(xv, &qty, span);
                let push = c.arr_push_discard(o.clone(), c.value_of(&x, &qinfo2), span);
                done.push(TStmt::Let(ov, Some(TExpr::new(TExprKind::ArrayLit(Vec::new()), oty, span))));
                done.push(TStmt::ForOf { id: id2, var: xv, arr: a2.clone(), body: vec![push] });
                done.extend(c.resolve_stmts(r2.clone(), &rinfo2, o, span));
            }
            let ok = vec![set_var(lv, dec, span), TStmt::If(is(left2.clone(), 0, span), done, Vec::new())];
            vec![TStmt::If(c.state_is(&q2, 2), vec![bad], ok)]
        });
        let empty = if void {
            TExpr::new(TExprKind::Bool(false), Type::Bool, span)
        } else {
            TExpr::new(TExprKind::ArrayLit(Vec::new()), out_ty.clone(), span)
        };
        let resolve_empty = self.resolve_stmts(r.clone(), &rinfo, empty, span);
        let len = TExpr::new(TExprKind::Rt("arr_len", vec![a.clone()]), Type::Int, span);
        let copy = self.arr_copy_of(var(ps[0], &aty, span), span);
        self.prog.funcs[fid as usize].body = vec![
            TStmt::Let(av, Some(copy)),
            TStmt::Let(rv, Some(self.new_promise(&rinfo, span))),
            TStmt::Let(lv, Some(len)),
            TStmt::If(is(left.clone(), 0, span), resolve_empty, Vec::new()),
            TStmt::ForOf { id, var: qv, arr: a, body: vec![self.then_stmt(q, k, span)] },
            TStmt::Return(Some(r)),
        ];
        fid
    }

    /// `Promise.all([p1, p2, …])` with promises of different types: a
    /// promise of the tuple of their values.
    fn promise_all_tuple(&mut self, items: Vec<TExpr>, span: Span) -> TExpr {
        let infos: Vec<PromiseInfo> = items.iter().map(|i| self.promise_of(&i.ty).cloned().expect("checked by the caller")).collect();
        if infos.iter().any(|i| i.ty == Type::Void) {
            self.err_help(
                code::ADVANCED_TYPE,
                span,
                "`Promise.all` of promises of different types cannot have a `Promise<void>`",
                "await the `Promise<void>` on its own",
            );
            return self.err_expr(span);
        }
        let tys: Vec<Type> = infos.iter().map(|i| i.ty.clone()).collect();
        let tsid = self.tuple_struct(&tys);
        let tty = Type::Struct(tsid);
        let rinfo = self.promise_info(&tty);
        let rty = Type::Struct(rinfo.sid);
        let mut pre = Vec::new();
        let qs: Vec<TExpr> = items
            .into_iter()
            .map(|i| {
                let ty = i.ty.clone();
                let v = self.temp(ty.clone());
                pre.push(TStmt::Let(v, Some(i)));
                var(v, &ty, span)
            })
            .collect();
        let rv = self.temp(rty.clone());
        let r = var(rv, &rty, span);
        let lv = self.temp(Type::Int);
        let left = var(lv, &Type::Int, span);
        pre.push(TStmt::Let(rv, Some(self.new_promise(&rinfo, span))));
        pre.push(TStmt::Let(lv, Some(int(qs.len() as i32, span))));
        for (i, q) in qs.iter().enumerate() {
            let (q2, qs2, infos2, r2, rinfo2, left2) = (q.clone(), qs.clone(), infos.clone(), r.clone(), rinfo.clone(), left.clone());
            let k = self.gen_closure("<all>", Vec::new(), span, |c, _, _| {
                let bad = c.reject_stmt(r2.clone(), c.error_of(&q2, infos2[i].sid), span);
                let dec = TExpr::new(TExprKind::Int2(IntOp::Sub, bx(left2.clone()), bx(int(1, span))), Type::Int, span);
                let vals: Vec<TExpr> = qs2.iter().zip(&infos2).map(|(q, info)| c.value_of(q, info)).collect();
                let tuple = TExpr::new(TExprKind::StructLit(tsid, vals), tty.clone(), span);
                let done = c.resolve_stmts(r2.clone(), &rinfo2, tuple, span);
                let ok = vec![set_var(lv, dec, span), TStmt::If(is(left2.clone(), 0, span), done, Vec::new())];
                vec![TStmt::If(c.state_is(&q2, 2), vec![bad], ok)]
            });
            pre.push(self.then_stmt(q.clone(), k, span));
        }
        TExpr::new(TExprKind::Block(pre, bx(r)), rty, span)
    }

    /// `new Promise<T>((resolve, reject) => …)`: the executor runs at once
    /// with a `resolve` and a `reject` function for the new promise; an
    /// exception in it rejects the promise.
    pub(super) fn new_promise_expr(&mut self, type_args: &[ast::TypeAnn], args: &[Expr], expected: Option<&Type>, span: Span) -> TExpr {
        let t = match (type_args, expected.and_then(|e| self.promise_of(e)).map(|i| i.ty.clone())) {
            ([t], _) => self.resolve_type(t),
            ([], Some(t)) => t,
            _ => {
                self.err_help(code::CANNOT_INFER, span, "cannot infer the value type of `new Promise`", "write `new Promise<T>((resolve, reject) => { ... })`");
                for a in args {
                    self.expr(a, None);
                }
                return self.err_expr(span);
            }
        };
        if args.len() != 1 {
            self.err(code::ARG_COUNT, span, "`new Promise` takes one function");
            return self.err_expr(span);
        }
        if t.is_error() {
            self.expr(&args[0], None);
            return self.err_expr(span);
        }
        let info = self.promise_info(&t);
        let pty = Type::Struct(info.sid);
        let err_ty = self.error_type();
        let res_params = if t == Type::Void { Vec::new() } else { vec![t.clone()] };
        let (ex, arity) = self.callback(&args[0], &[void_fn(res_params.clone()), void_fn(vec![err_ty.clone()])], Some(Type::Void));
        let pv = self.temp(pty.clone());
        let p = var(pv, &pty, span);
        let (p2, info2) = (p.clone(), info.clone());
        let res = self.gen_closure("<resolve>", res_params, span, |c, _, vs| {
            let v = match vs.first() {
                Some(v) => var(*v, &info2.ty, span),
                None => TExpr::new(TExprKind::Bool(false), Type::Bool, span),
            };
            c.resolve_stmts(p2, &info2, v, span)
        });
        let p3 = p.clone();
        let rej = self.gen_closure("<reject>", vec![err_ty.clone()], span, |c, _, vs| vec![c.reject_stmt(p3, var(vs[0], &err_ty, span), span)]);
        let call = self.call_cb(&ex, arity, vec![res, rej], span);
        let owner = self.fx.func;
        let mut stmts = vec![TStmt::Let(pv, Some(self.new_promise(&info, span)))];
        let body = self.reject_on_throw(owner, &p, vec![TStmt::Expr(call)], span);
        // The catch variable belongs to the current function and loop.
        if let [TStmt::Try { catch: Some((ev, _)), .. }] = &body[..] {
            let lp = self.fx.loops.last().copied();
            self.prog.vars[*ev as usize].in_loop = lp;
        }
        stmts.extend(body);
        TExpr::new(TExprKind::Block(stmts, bx(p)), pty, span)
    }
}
