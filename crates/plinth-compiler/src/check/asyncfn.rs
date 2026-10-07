//! `async` functions, `await` and `Promise<T>` (SPEC.md §4.5).
//!
//! # Promises
//!
//! A `Promise<T>` is a struct with the fields `#state` (0 pending, 1
//! fulfilled, 2 rejected), `#error` (`Error | null`), `#waiters` (an array
//! of `() => void` continuations), `#handled` and `#value` (`T`; a
//! `boolean` for `Promise<void>`). The names start with `#`, so app code
//! cannot read them. The fields before `#value` have the same layout in
//! every promise, so `Promise<void>` is a common view of all of them. For
//! each `T` the checker makes three functions:
//!
//! - `resolve(p, v)` and `reject(p, e)` settle a pending promise and queue
//!   its waiters as microtasks. A rejection with no waiters goes to a list
//!   of possibly unhandled rejections.
//! - `then(p, k)` adds the continuation `k`, or queues it at once when `p`
//!   has settled (and marks a rejected `p` as handled).
//!
//! The microtask queue is app code too (`make_async_rt`): a module
//! variable and a `drain` function, which the app gives to the runtime
//! (`set_drain`, core 1.10) when it first queues something. The runtime
//! calls the drain after each event, before the reactive flush, so a
//! signal that a continuation sets updates the UI in the same commit. The
//! drain then reports each rejection that nothing handled (`report`).
//! An app with no `async` code pays nothing in the runtime.
//!
//! # The continuation transform
//!
//! After the checker checks the body of an `async` function, it rewrites
//! the body into closures: the code after an `await` becomes a
//! continuation closure that `then` registers. Variables that a
//! continuation uses move to heap frames (the normal capture rule), so
//! they live while the function waits. A pending continuation is
//! reachable from the promise it waits on, and that promise from the host
//! request or the timer that settles it, so the GC keeps it (SPEC.md §5.4).
//!
//! `await` can be a statement of its own (`await p;`), the value of a
//! variable (`const x = await p;`), of an assignment (`x = await p;`), or
//! of a `return`. An `await` deeper in an expression is first moved out
//! (`lin`): the parts that run before it go into variables, and `&&`,
//! `||` and `?:` become `if` statements. Statements with an `await` can be
//! in `if`, `while`, `do…while`, `for`, `for…of`, `switch`, blocks and
//! `try`/`catch`/`finally`. A `do…while` becomes a `while` with a flag; a
//! `switch` becomes `if` statements that select and run the cases
//! (`async_switch`); a `finally` body becomes a closure that the end of
//! the `try`, an exception, and a `return`, `break` or `continue` call
//! (`async_finally`).
//! A loop with an `await` becomes a loop closure: an iteration that does
//! not wait stays in the Wasm loop, and a continuation queues the loop
//! closure again.
//!
//! Each continuation runs inside a `try`: an exception rejects the
//! function's promise, or goes to the closure of the enclosing `catch`.
//! An `await` of a rejected promise throws its error.

use super::Checker;
use crate::codegen::{expr_children, expr_children_mut, stmt_parts, walk_stmt};
use crate::diag::{Span, code};
use crate::tir::*;
use crate::types::{Field, FuncType, StructDef, StructId, Type};
use std::rc::Rc;

pub(super) const STATE: u32 = 0;
pub(super) const ERROR: u32 = 1;
const WAITERS: u32 = 2;
const HANDLED: u32 = 3;
pub(super) const VALUE: u32 = 4;

/// One `Promise<T>`: its struct and helper functions.
#[derive(Clone, Debug)]
pub(crate) struct PromiseInfo {
    /// `T`.
    pub ty: Type,
    pub sid: StructId,
    pub(super) resolve: FuncId,
}

pub(super) fn bx(e: TExpr) -> Box<TExpr> {
    Box::new(e)
}

pub(super) fn int(n: i32, span: Span) -> TExpr {
    TExpr::new(TExprKind::Num(n as f64), Type::Int, span)
}

pub(super) fn waiter_ty() -> Type {
    Type::Func(Rc::new(FuncType { params: Vec::new(), required: 0, ret: Type::Void }))
}

/// The `#value` type of `Promise<t>`.
pub(super) fn value_ty(t: &Type) -> Type {
    if *t == Type::Void { Type::Bool } else { t.clone() }
}

pub(super) fn void_stmt(kind: TExprKind, span: Span) -> TStmt {
    TStmt::Expr(TExpr::new(kind, Type::Void, span))
}

/// `v = e;`
pub(super) fn set_var(v: VarId, e: TExpr, span: Span) -> TStmt {
    let ty = e.ty.clone();
    TStmt::Expr(TExpr::new(TExprKind::Assign(Place::Var(v), bx(e)), ty, span))
}

/// `e === n` on `int` values.
pub(super) fn is(e: TExpr, n: i32, span: Span) -> TExpr {
    TExpr::new(TExprKind::Cmp(CmpOp::Eq, EqKind::I32, bx(e), bx(int(n, span))), Type::Bool, span)
}

/// The app's microtask machinery (`make_async_rt`).
#[derive(Clone, Debug)]
pub(crate) struct AsyncRt {
    unhandled: VarId,
    init: FuncId,
    enqueue: FuncId,
    /// `Promise<void>`, the common view of every promise.
    base: StructId,
    /// `settle(p, state)`, `reject(p, e)` and `then(p, k)` on the common
    /// view: one of each for all promise types.
    settle: FuncId,
    reject: FuncId,
    then: FuncId,
}

/// The tail of a statement list: what runs when control reaches its end.
#[derive(Clone)]
enum Tail {
    /// The end of the function body: resolve the promise (`void` only).
    Resolve,
    /// Call this continuation closure.
    Call(VarId),
    /// The end of a loop body in a continuation: run the update, then queue
    /// the loop closure again.
    Next(Box<LoopCx>),
    /// The end of a loop body inside the Wasm loop: nothing to do.
    FallThrough(Box<LoopCx>),
}

#[derive(Clone)]
struct LoopCx {
    /// The loop closure.
    lv: VarId,
    update: Option<TExpr>,
    /// What runs after the loop (for `break` in a continuation).
    brk: Tail,
    /// True while the code is inside the Wasm loop in the loop closure.
    in_tir: bool,
}

/// The innermost `try`/`finally` with an `await`: a `return`, `break` or
/// `continue` that leaves it records a completion kind in `ck` (2, 3, 4;
/// 1 is a throw, 0 the normal end) and calls the `finally` closure.
struct FinCx {
    ck: VarId,
    /// The value of a `return` (not for `Promise<void>`).
    cv: Option<VarId>,
    /// The `finally` closure.
    fin: VarId,
    /// A `return`, `break`, `continue` went through it.
    used: std::cell::Cell<[bool; 3]>,
}

#[derive(Clone)]
struct Ctx {
    owner: FuncId,
    tail: Tail,
    lcx: Option<LoopCx>,
    /// The closures of the enclosing `catch` blocks, innermost last.
    handlers: Vec<VarId>,
    /// The code after the innermost transformed `switch` (where its `break`
    /// goes), unless a loop is nearer.
    sw: Option<Box<Tail>>,
    fin: Option<Rc<FinCx>>,
    /// `break` and `continue` at the top go through `fin` (no loop or
    /// `switch` is between).
    fin_brk: bool,
    fin_cont: bool,
    /// Inside a synchronous `try`/`finally` in the `try` blocks of `fin`:
    /// the flag variable of the outermost such `try`, and whether a
    /// `return`, `break` or `continue` used it. A statement that leaves
    /// `fin` sets the flag and returns; the synchronous `finally` blocks
    /// run first, and the last one calls the `finally` closure of `fin`.
    sync_fin: Option<(VarId, Rc<std::cell::Cell<bool>>)>,
}

/// A tail for code in a new closure: it is not inside the Wasm loop.
fn relocate_tail(t: &Tail) -> Tail {
    match t {
        Tail::FallThrough(l) => Tail::Next(Box::new(LoopCx { in_tir: false, ..(**l).clone() })),
        t => t.clone(),
    }
}

impl Ctx {
    fn new(owner: FuncId, tail: Tail) -> Ctx {
        Ctx { owner, tail, lcx: None, handlers: Vec::new(), sw: None, fin: None, fin_brk: false, fin_cont: false, sync_fin: None }
    }

    /// The context for code moved into a new closure `owner`.
    fn relocate(&self, owner: FuncId) -> Ctx {
        Ctx {
            owner,
            tail: relocate_tail(&self.tail),
            lcx: self.lcx.clone().map(|l| LoopCx { in_tir: false, ..l }),
            sw: self.sw.as_ref().map(|t| Box::new(relocate_tail(t))),
            ..self.clone()
        }
    }
}

/// The state of one `async` function's transform.
struct Acx {
    p: VarId,
    info: PromiseInfo,
    span: Span,
}

fn has_await_expr(e: &TExpr) -> bool {
    if matches!(e.kind, TExprKind::Await(_)) {
        return true;
    }
    if let TExprKind::Block(stmts, _) = &e.kind
        && stmts.iter().any(has_await)
    {
        return true;
    }
    expr_children(e).into_iter().any(has_await_expr)
}

fn has_await(s: &TStmt) -> bool {
    let mut found = false;
    walk_stmt(s, &mut |_| {}, &mut |e| {
        if matches!(e.kind, TExprKind::Await(_)) {
            found = true;
        }
    });
    found
}

/// The span of the first `await` in `s`.
fn await_span(s: &TStmt, fallback: Span) -> Span {
    let mut span = None;
    walk_stmt(s, &mut |_| {}, &mut |e| {
        if span.is_none() && matches!(e.kind, TExprKind::Await(_)) {
            span = Some(e.span);
        }
    });
    span.unwrap_or(fallback)
}

/// The operand of an `await` at the top of `e` (through conversions and
/// the value of an assignment).
fn top_await(e: &TExpr) -> Option<&TExpr> {
    match &e.kind {
        TExprKind::Await(inner) => Some(inner),
        TExprKind::Coerce(_, x) => top_await(x),
        TExprKind::Assign(_, v) => top_await(v),
        _ => None,
    }
}

/// Replaces the `await` that `top_await` found with `with`.
fn replace_top_await(e: &mut TExpr, with: TExpr) {
    match &mut e.kind {
        TExprKind::Await(_) => *e = with,
        TExprKind::Coerce(_, x) => replace_top_await(x, with),
        TExprKind::Assign(_, v) => replace_top_await(v, with),
        _ => unreachable!("checked by top_await"),
    }
}

impl Checker<'_> {
    // -- Promise types ---------------------------------------------------------

    /// `Promise<t>`.
    pub(crate) fn promise_type(&mut self, t: &Type) -> Type {
        Type::Struct(self.promise_info(t).sid)
    }

    /// The `PromiseInfo` of a promise type, or `None` for another type.
    pub(crate) fn promise_of(&self, t: &Type) -> Option<&PromiseInfo> {
        match t {
            Type::Struct(sid) => self.promises.iter().find(|p| p.sid == *sid),
            _ => None,
        }
    }

    /// The `T` of a declared `Promise<T>` return type of an `async`
    /// function, or an error.
    pub(crate) fn promise_inner_or_err(&mut self, t: &Type, span: Span) -> Type {
        if t.is_error() {
            return Type::Error;
        }
        match self.promise_of(t) {
            Some(info) => info.ty.clone(),
            None => {
                let msg = format!("an `async` function returns a `Promise`, not `{}`", self.show(t));
                self.err_help(code::TYPE_MISMATCH, span, msg, "write the return type as `Promise<T>`");
                Type::Error
            }
        }
    }

    pub(crate) fn promise_info(&mut self, t: &Type) -> PromiseInfo {
        if let Some(info) = self.promises.iter().find(|p| p.ty == *t) {
            return info.clone();
        }
        // `Promise<void>` first: it is the common view of every promise
        // (the fields before `#value` have the same layout in all of them).
        if *t != Type::Void && self.promises.is_empty() {
            self.promise_info(&Type::Void);
        }
        let span = Span::default();
        let sid = self.prog.structs.len() as StructId;
        let name = format!("Promise<{}>", self.show(t));
        let fields = vec![
            Field { name: "#state".into(), ty: Type::Int, optional: false },
            Field { name: "#error".into(), ty: self.error_type().nullable(), optional: false },
            Field { name: "#waiters".into(), ty: Type::Array(Box::new(waiter_ty())), optional: false },
            Field { name: "#handled".into(), ty: Type::Bool, optional: false },
            Field { name: "#value".into(), ty: value_ty(t), optional: false },
        ];
        self.prog.structs.push(StructDef { name: name.clone(), fields });
        let pty = Type::Struct(sid);
        if self.async_rt.is_none() {
            self.make_async_rt(sid);
        }

        // resolve(p, v): `if (p.#state === 0) { p.#value = v; settle(p, 1); }`
        let settle = self.async_rt.as_ref().expect("made above").settle;
        let (resolve, rv) = self.helper_fn(&format!("{name}.resolve"), &[("p", pty.clone()), ("v", value_ty(t))], Type::Void);
        let p = TExpr::new(TExprKind::Var(rv[0]), pty.clone(), span);
        let v = TExpr::new(TExprKind::Var(rv[1]), value_ty(t), span);
        let body = vec![TStmt::If(
            is(self.pfield(&p, sid, STATE, Type::Int), 0, span),
            vec![self.set_pfield(p.clone(), sid, VALUE, v), void_stmt(TExprKind::Call(settle, vec![self.as_base(p), int(1, span)]), span)],
            Vec::new(),
        )];
        self.prog.funcs[resolve as usize].body = body;

        let info = PromiseInfo { ty: t.clone(), sid, resolve };
        self.promises.push(info.clone());
        info
    }

    /// The app's microtask queue (SPEC.md §4.5): two module variables (the
    /// queue, and the rejected promises that had no waiter), `init` (makes
    /// both and gives the runtime the drain closure, once), `enqueue(k)`
    /// and `drain()`. `base` is `Promise<void>`, the common view of every
    /// promise.
    fn make_async_rt(&mut self, base: StructId) {
        let span = Span::default();
        let qty = Type::Array(Box::new(waiter_ty()));
        let bty = Type::Struct(base);
        let uty = Type::Array(Box::new(bty.clone()));
        let module_var = |c: &mut Self, name: &str, ty: Type| {
            c.prog.new_var(VarInfo { name: name.into(), ty, owner: 0, mutable: true, module: Some(0), in_loop: None, captured: false })
        };
        let queue = module_var(self, "$microtasks", qty.clone());
        let unhandled = module_var(self, "$unhandled", uty.clone());
        let q = TExpr::new(TExprKind::Var(queue), qty.clone(), span);
        let u = TExpr::new(TExprKind::Var(unhandled), uty.clone(), span);
        let assign = |v: VarId, e: TExpr| {
            let ty = e.ty.clone();
            TStmt::Expr(TExpr::new(TExprKind::Assign(Place::Var(v), bx(e)), ty, span))
        };
        let empty = |ty: &Type| TExpr::new(TExprKind::ArrayLit(Vec::new()), ty.clone(), span);

        // drain(): run the queue until it is empty, then report.
        let (drain, _) = self.helper_fn("<drain>", &[], Type::Int);
        let n = self.var_in(drain, "$n", Type::Int, None);
        let n_r = TExpr::new(TExprKind::Var(n), Type::Int, span);
        let batch = self.var_in(drain, "$batch", qty.clone(), None);
        let id = self.prog.new_loop();
        let k = self.var_in(drain, "$k", waiter_ty(), Some(id));
        let len = |a: TExpr| TExpr::new(TExprKind::Rt("arr_len", vec![a]), Type::Int, span);
        let more = TExpr::new(TExprKind::Cmp(CmpOp::Gt, EqKind::I32, bx(len(q.clone())), bx(int(0, span))), Type::Bool, span);
        let outer = self.prog.new_loop();
        let run = TStmt::Loop {
            id: outer,
            cond: Some(more),
            test_after: false,
            update: None,
            body: vec![
                TStmt::Let(batch, Some(q.clone())),
                assign(queue, empty(&qty)),
                TStmt::ForOf {
                    id,
                    var: k,
                    arr: TExpr::new(TExprKind::Var(batch), qty.clone(), span),
                    body: vec![
                        void_stmt(TExprKind::CallClosure(bx(TExpr::new(TExprKind::Var(k), waiter_ty(), span)), Vec::new()), span),
                        assign(n, TExpr::new(TExprKind::Int2(IntOp::Add, bx(n_r.clone()), bx(int(1, span))), Type::Int, span)),
                    ],
                },
            ],
        };
        self.prog.vars[batch as usize].in_loop = Some(outer);
        let us = self.var_in(drain, "$us", uty.clone(), None);
        let rid = self.prog.new_loop();
        let pv = self.var_in(drain, "$p", bty.clone(), Some(rid));
        let p = TExpr::new(TExprKind::Var(pv), bty.clone(), span);
        let ec = self.error_class;
        let err = TExpr::new(
            TExprKind::Coerce(Coercion::Retag, bx(self.pfield(&p, base, ERROR, self.error_type().nullable()))),
            self.error_type(),
            span,
        );
        let s = |t: &str| TExpr::new(TExprKind::Str(t.to_owned()), Type::String, span);
        let cat = |a: TExpr, b: TExpr| TExpr::new(TExprKind::Concat(bx(a), bx(b)), Type::String, span);
        let name = TExpr::new(TExprKind::Field(bx(err.clone()), ec, 0), Type::String, span);
        let msg = TExpr::new(TExprKind::Field(bx(err), ec, 1), Type::String, span);
        let text = cat(cat(cat(s("Uncaught (in promise) "), name), s(": ")), msg);
        let not_handled = TExpr::new(TExprKind::Not(bx(self.pfield(&p, base, HANDLED, Type::Bool))), Type::Bool, span);
        let body = vec![
            TStmt::Let(n, Some(int(0, span))),
            run,
            TStmt::Let(us, Some(u.clone())),
            assign(unhandled, empty(&uty)),
            TStmt::ForOf {
                id: rid,
                var: pv,
                arr: TExpr::new(TExprKind::Var(us), uty.clone(), span),
                body: vec![TStmt::If(not_handled, vec![void_stmt(TExprKind::Rt("report", vec![text]), span)], Vec::new())],
            },
            TStmt::Return(Some(n_r)),
        ];
        self.prog.funcs[drain as usize].body = body;

        // init(): once, make the queue and give the runtime the drain.
        let (init, _) = self.helper_fn("<async init>", &[], Type::Void);
        let d = self.var_in(init, "$d", Type::Func(Rc::new(FuncType { params: Vec::new(), required: 0, ret: Type::Int })), None);
        let d_r = TExpr::new(TExprKind::Var(d), self.prog.vars[d as usize].ty.clone(), span);
        let thunk = TExpr::new(TExprKind::ThunkOf(ThunkSig { params: Vec::new(), ret: crate::types::Repr::I32 }), Type::Bool, span);
        let body = vec![TStmt::If(
            TExpr::new(TExprKind::IsNull(bx(q.clone())), Type::Bool, span),
            vec![
                assign(queue, empty(&qty)),
                assign(unhandled, empty(&uty)),
                TStmt::Let(d, Some(TExpr::new(TExprKind::Closure(drain), d_r.ty.clone(), span))),
                void_stmt(TExprKind::Rt("root", vec![d_r.clone()]), span),
                void_stmt(TExprKind::Rt("set_drain", vec![thunk, d_r]), span),
            ],
            Vec::new(),
        )];
        self.prog.funcs[init as usize].body = body;

        // enqueue(k)
        let (enqueue, ev) = self.helper_fn("<enqueue>", &[("k", waiter_ty())], Type::Void);
        let k = TExpr::new(TExprKind::Var(ev[0]), waiter_ty(), span);
        self.prog.funcs[enqueue as usize].body = vec![
            void_stmt(TExprKind::Call(init, Vec::new()), span),
            TStmt::Expr(TExpr::new(TExprKind::Rt("arr_push_i32", vec![q, k]), Type::Int, span)),
        ];
        self.async_rt = Some(AsyncRt { unhandled, init, enqueue, base, settle: 0, reject: 0, then: 0 });

        // settle(p, s)
        let (settle, sv) = self.helper_fn("<settle>", &[("p", bty.clone()), ("s", Type::Int)], Type::Void);
        let body = self.settle_body(sv[0], sv[1], base);
        self.prog.funcs[settle as usize].body = body;

        // reject(p, e): `if (p.#state === 0) { p.#error = e; settle(p, 2); }`
        let err_ty = self.error_type();
        let (reject, jv) = self.helper_fn("<reject>", &[("p", bty.clone()), ("e", err_ty.clone())], Type::Void);
        let p = TExpr::new(TExprKind::Var(jv[0]), bty.clone(), span);
        let e = TExpr::new(TExprKind::Var(jv[1]), err_ty.clone(), span);
        let as_nullable = TExpr::new(TExprKind::Coerce(Coercion::Retag, bx(e)), err_ty.nullable(), span);
        self.prog.funcs[reject as usize].body = vec![TStmt::If(
            is(self.pfield(&p, base, STATE, Type::Int), 0, span),
            vec![self.set_pfield(p.clone(), base, ERROR, as_nullable), void_stmt(TExprKind::Call(settle, vec![p, int(2, span)]), span)],
            Vec::new(),
        )];

        // then(p, k): wait for `p`, or queue `k` at once when `p` has settled.
        let (then, tv) = self.helper_fn("<then>", &[("p", bty.clone()), ("k", waiter_ty())], Type::Void);
        let p = TExpr::new(TExprKind::Var(tv[0]), bty.clone(), span);
        let k = TExpr::new(TExprKind::Var(tv[1]), waiter_ty(), span);
        let state = self.pfield(&p, base, STATE, Type::Int);
        let waiters = self.pfield(&p, base, WAITERS, Type::Array(Box::new(waiter_ty())));
        self.prog.funcs[then as usize].body = vec![TStmt::If(
            is(state, 0, span),
            vec![TStmt::Expr(TExpr::new(TExprKind::Rt("arr_push_i32", vec![waiters, k.clone()]), Type::Int, span))],
            vec![self.set_pfield(p.clone(), base, HANDLED, TExpr::new(TExprKind::Bool(true), Type::Bool, span)), self.enqueue(k, span)],
        )];
        self.async_rt = Some(AsyncRt { unhandled, init, enqueue, base, settle, reject, then });
    }

    /// The shared `then(p, k)` and `reject(p, e)`.
    pub(super) fn promise_then_reject(&self) -> (FuncId, FuncId) {
        let rt = self.async_rt.as_ref().expect("made with the first promise");
        (rt.then, rt.reject)
    }

    /// `e` as the common view `Promise<void>`.
    pub(super) fn as_base(&self, e: TExpr) -> TExpr {
        let base = self.async_rt.as_ref().expect("made with the first promise").base;
        if e.ty == Type::Struct(base) {
            return e;
        }
        let span = e.span;
        TExpr::new(TExprKind::Coerce(Coercion::Retag, bx(e)), Type::Struct(base), span)
    }

    /// `enqueue(k)`: runs `k` as a microtask.
    fn enqueue(&self, k: TExpr, span: Span) -> TStmt {
        let rt = self.async_rt.as_ref().expect("made with the first promise");
        void_stmt(TExprKind::Call(rt.enqueue, vec![k]), span)
    }

    /// A top-level helper function with these parameters.
    pub(super) fn helper_fn(&mut self, name: &str, params: &[(&str, Type)], ret: Type) -> (FuncId, Vec<VarId>) {
        let fid = self.prog.new_func(FuncDef {
            name: name.to_owned(),
            kind: FuncKind::TopLevel,
            params: Vec::new(),
            ret,
            body: Vec::new(),
            span: Span::default(),
        });
        let vars: Vec<VarId> = params.iter().map(|(n, t)| self.var_in(fid, n, t.clone(), None)).collect();
        self.prog.funcs[fid as usize].params = vars.clone();
        (fid, vars)
    }

    pub(super) fn var_in(&mut self, owner: FuncId, name: &str, ty: Type, in_loop: Option<LoopId>) -> VarId {
        self.prog.new_var(VarInfo { name: name.to_owned(), ty, owner, mutable: true, module: None, in_loop, captured: false })
    }

    pub(super) fn pfield(&self, p: &TExpr, sid: StructId, idx: u32, ty: Type) -> TExpr {
        TExpr::new(TExprKind::Field(bx(p.clone()), sid, idx), ty, p.span)
    }

    pub(super) fn set_pfield(&self, p: TExpr, sid: StructId, idx: u32, v: TExpr) -> TStmt {
        let span = p.span;
        let ty = v.ty.clone();
        TStmt::Expr(TExpr::new(TExprKind::Assign(Place::Field(bx(p), sid, idx), bx(v)), ty, span))
    }

    /// The body of `settle(p, s)`: `p.#state = s`, then queue the waiters.
    /// A rejection with no waiters goes to the unhandled list; the drain
    /// reports it unless an `await` handles it first.
    fn settle_body(&mut self, pv: VarId, sv: VarId, sid: StructId) -> Vec<TStmt> {
        let span = Span::default();
        let fid = self.prog.vars[pv as usize].owner;
        let p = TExpr::new(TExprKind::Var(pv), Type::Struct(sid), span);
        let st = TExpr::new(TExprKind::Var(sv), Type::Int, span);
        let arr_ty = Type::Array(Box::new(waiter_ty()));
        let ws = self.var_in(fid, "$ws", arr_ty.clone(), None);
        let ws_r = TExpr::new(TExprKind::Var(ws), arr_ty.clone(), span);
        let id = self.prog.new_loop();
        let w = self.var_in(fid, "$w", waiter_ty(), Some(id));
        let rt = self.async_rt.clone().expect("made with the first promise");
        let len = TExpr::new(TExprKind::Rt("arr_len", vec![ws_r.clone()]), Type::Int, span);
        let empty = TExpr::new(TExprKind::Cmp(CmpOp::Eq, EqKind::I32, bx(len), bx(int(0, span))), Type::Bool, span);
        let unhandled = TExpr::new(TExprKind::And(bx(is(st.clone(), 2, span)), bx(empty)), Type::Bool, span);
        let uty = Type::Array(Box::new(Type::Struct(rt.base)));
        let u = TExpr::new(TExprKind::Var(rt.unhandled), uty, span);
        let enq = self.enqueue(TExpr::new(TExprKind::Var(w), waiter_ty(), span), span);
        vec![
            self.set_pfield(p.clone(), sid, STATE, st),
            TStmt::Let(ws, Some(self.pfield(&p, sid, WAITERS, arr_ty.clone()))),
            self.set_pfield(p.clone(), sid, WAITERS, TExpr::new(TExprKind::ArrayLit(Vec::new()), arr_ty.clone(), span)),
            TStmt::If(
                unhandled,
                vec![
                    void_stmt(TExprKind::Call(rt.init, Vec::new()), span),
                    TStmt::Expr(TExpr::new(TExprKind::Rt("arr_push_i32", vec![u, p]), Type::Int, span)),
                ],
                Vec::new(),
            ),
            TStmt::ForOf { id, var: w, arr: ws_r, body: vec![enq] },
        ]
    }

    /// A new pending `Promise<info.ty>`.
    pub(super) fn new_promise(&self, info: &PromiseInfo, span: Span) -> TExpr {
        let fields = vec![
            int(0, span),
            TExpr::new(TExprKind::Null, self.error_type().nullable(), span),
            TExpr::new(TExprKind::ArrayLit(Vec::new()), Type::Array(Box::new(waiter_ty())), span),
            TExpr::new(TExprKind::Bool(false), Type::Bool, span),
            TExpr::new(TExprKind::Null, value_ty(&info.ty), span),
        ];
        TExpr::new(TExprKind::StructLit(info.sid, fields), Type::Struct(info.sid), span)
    }

    /// The promise form of a host call (SPEC.md §4.5): `call` gets a
    /// callback that resolves a new `Promise<t>`, and the expression is
    /// that promise. A callback for `t = void` takes no argument.
    pub(crate) fn host_promise(&mut self, t: Type, span: Span, call: impl FnOnce(&mut Self, TExpr) -> TExpr) -> TExpr {
        let info = self.promise_info(&t);
        let pty = Type::Struct(info.sid);
        let p = self.temp(pty.clone());
        let fid = self.prog.new_func(FuncDef {
            name: "<resolve>".into(),
            kind: FuncKind::Closure,
            params: Vec::new(),
            ret: Type::Void,
            body: Vec::new(),
            span,
        });
        let (params, value) = if t == Type::Void {
            (Vec::new(), TExpr::new(TExprKind::Bool(false), Type::Bool, span))
        } else {
            let v = self.var_in(fid, "v", t.clone(), None);
            (vec![v], TExpr::new(TExprKind::Var(v), t.clone(), span))
        };
        self.prog.funcs[fid as usize].params = params.clone();
        let p_r = TExpr::new(TExprKind::Var(p), pty.clone(), span);
        self.prog.funcs[fid as usize].body = vec![void_stmt(TExprKind::Call(info.resolve, vec![p_r.clone(), value]), span)];
        let ft = FuncType { params: params.iter().map(|v| self.prog.vars[*v as usize].ty.clone()).collect(), required: params.len(), ret: Type::Void };
        let cb = TExpr::new(TExprKind::Closure(fid), Type::Func(Rc::new(ft)), span);
        let host = call(self, cb);
        TExpr::new(TExprKind::Block(vec![TStmt::Let(p, Some(self.new_promise(&info, span))), TStmt::Expr(host)], bx(p_r)), pty, span)
    }

    // -- The transform ---------------------------------------------------------

    /// Rewrites the checked body of the `async` function `fid` (SPEC.md
    /// §4.5). The new body makes the promise, runs the start closure, and
    /// returns the promise (or nothing, when `detached`).
    pub(crate) fn async_body(&mut self, fid: FuncId, body: Vec<TStmt>, t: &Type, detached: bool, span: Span) -> Vec<TStmt> {
        let info = self.promise_info(t);
        let pty = Type::Struct(info.sid);
        let p = self.var_in(fid, "$promise", pty.clone(), None);
        let acx = Acx { p, info: info.clone(), span };
        let cx = Ctx::new(fid, Tail::Resolve);
        let start = self.make_closure(&acx, &cx, "<async start>", Vec::new(), |c, kcx| c.block(&acx, body, kcx));
        let p_r = TExpr::new(TExprKind::Var(p), pty, span);
        vec![
            TStmt::Let(p, Some(self.new_promise(&info, span))),
            TStmt::Expr(TExpr::new(TExprKind::CallClosure(bx(start), Vec::new()), Type::Void, span)),
            TStmt::Return(if detached { None } else { Some(p_r) }),
        ]
    }

    /// A new continuation closure: its body is `build(context)`, wrapped
    /// so an exception goes to the enclosing handler. The variables that
    /// the body declares move into it.
    fn make_closure(
        &mut self,
        acx: &Acx,
        cx: &Ctx,
        name: &str,
        params: Vec<VarId>,
        build: impl FnOnce(&mut Self, &Ctx) -> Vec<TStmt>,
    ) -> TExpr {
        let fid = self.prog.new_func(FuncDef {
            name: name.to_owned(),
            kind: FuncKind::Closure,
            params: params.clone(),
            ret: Type::Void,
            body: Vec::new(),
            span: acx.span,
        });
        for v in &params {
            let info = &mut self.prog.vars[*v as usize];
            info.owner = fid;
            info.in_loop = None;
        }
        let kcx = cx.relocate(fid);
        let body = build(self, &kcx);
        let body = self.wrap(acx, body, &cx.handlers);
        reown(&mut self.prog, &body, fid, None);
        self.prog.funcs[fid as usize].body = body;
        let ft = FuncType {
            params: params.iter().map(|v| self.prog.vars[*v as usize].ty.clone()).collect(),
            required: params.len(),
            ret: Type::Void,
        };
        TExpr::new(TExprKind::Closure(fid), Type::Func(Rc::new(ft)), acx.span)
    }

    /// `try { body } catch (e) { handler(e) }`, where the handler is the
    /// innermost `catch` closure or the rejection of the promise.
    fn wrap(&mut self, acx: &Acx, body: Vec<TStmt>, handlers: &[VarId]) -> Vec<TStmt> {
        let span = acx.span;
        let err_ty = self.error_type();
        let ev = self.var_in(0, "$e", err_ty.clone(), None);
        let e = TExpr::new(TExprKind::Var(ev), err_ty.clone(), span);
        let dispatch = match handlers.last() {
            Some(h) => {
                let hty = self.prog.vars[*h as usize].ty.clone();
                TExprKind::CallClosure(bx(TExpr::new(TExprKind::Var(*h), hty, span)), vec![e])
            }
            None => {
                let reject = self.async_rt.as_ref().expect("made with the first promise").reject;
                let p = self.as_base(TExpr::new(TExprKind::Var(acx.p), Type::Struct(acx.info.sid), span));
                TExprKind::Call(reject, vec![p, e])
            }
        };
        vec![TStmt::Try { body, catch: Some((ev, vec![void_stmt(dispatch, span)])), finally: None }]
    }

    fn emit_tail(&mut self, acx: &Acx, tail: &Tail) -> Vec<TStmt> {
        let span = acx.span;
        match tail {
            Tail::Resolve => {
                let mut out = Vec::new();
                if acx.info.ty == Type::Void {
                    let p = TExpr::new(TExprKind::Var(acx.p), Type::Struct(acx.info.sid), span);
                    out.push(void_stmt(TExprKind::Call(acx.info.resolve, vec![p, TExpr::new(TExprKind::Bool(false), Type::Bool, span)]), span));
                }
                out.push(TStmt::Return(None));
                out
            }
            Tail::Call(k) => vec![self.call_var(*k, span), TStmt::Return(None)],
            Tail::Next(l) => {
                let mut out = Vec::new();
                if let Some(u) = &l.update {
                    out.push(TStmt::Expr(u.clone()));
                }
                // Queue the next iteration instead of calling it: when this
                // code runs without a real wait (an `if` whose branch did
                // not `await`), a direct call would grow the stack once per
                // iteration.
                let lty = self.prog.vars[l.lv as usize].ty.clone();
                out.push(self.enqueue(TExpr::new(TExprKind::Var(l.lv), lty, span), span));
                out.push(TStmt::Return(None));
                out
            }
            Tail::FallThrough(_) => Vec::new(),
        }
    }

    fn call_var(&self, v: VarId, span: Span) -> TStmt {
        let ty = self.prog.vars[v as usize].ty.clone();
        void_stmt(TExprKind::CallClosure(bx(TExpr::new(TExprKind::Var(v), ty, span)), Vec::new()), span)
    }

    /// Transforms a statement list; control that reaches its end runs the
    /// tail of `cx`.
    fn block(&mut self, acx: &Acx, stmts: Vec<TStmt>, cx: &Ctx) -> Vec<TStmt> {
        let mut out = Vec::new();
        let mut it = stmts.into_iter();
        while let Some(s) = it.next() {
            if !has_await(&s) {
                out.extend(self.sync_stmt(acx, s, cx, 0, 0));
                continue;
            }
            let rest: Vec<TStmt> = it.collect();
            let simple = match &s {
                TStmt::Let(_, Some(e)) | TStmt::Expr(e) | TStmt::Return(Some(e)) => top_await(e).is_some_and(|pe| !has_await_expr(pe)) && {
                    let mut probe = e.clone();
                    replace_top_await(&mut probe, TExpr::new(TExprKind::Null, Type::Error, e.span));
                    !has_await_expr(&probe)
                },
                _ => false,
            };
            if simple {
                out.extend(self.await_stmt(acx, s, rest, cx));
                return out;
            }
            // An `await` inside an expression: evaluate the parts before
            // it into variables, then `const t = await p;`.
            if let Some(expanded) = self.linearize_stmt(cx, &s) {
                out.extend(self.block(acx, expanded.into_iter().chain(rest).collect(), cx));
                return out;
            }
            let after = if rest.is_empty() {
                cx.tail.clone()
            } else {
                let k = self.make_closure(acx, cx, "<async rest>", Vec::new(), |c, kcx| c.block(acx, rest, kcx));
                let kv = self.var_in(cx.owner, "$k", waiter_ty(), None);
                out.push(TStmt::Let(kv, Some(k)));
                Tail::Call(kv)
            };
            let ccx = Ctx { tail: after, ..cx.clone() };
            out.extend(self.compound(acx, s, &ccx));
            return out;
        }
        out.extend(self.emit_tail(acx, &cx.tail));
        out
    }

    /// A statement with no `await`: `return` resolves the promise, and a
    /// `break`/`continue` of a transformed loop outside its Wasm loop
    /// becomes a call of the code after the loop or of the loop closure.
    fn sync_stmt(&mut self, acx: &Acx, s: TStmt, cx: &Ctx, loops: u32, brks: u32) -> Vec<TStmt> {
        let span = acx.span;
        let outside = cx.lcx.as_ref().filter(|l| !l.in_tir).cloned();
        let fin = cx.fin.is_some();
        match s {
            TStmt::Return(e) if fin => self.leave_via_finally(acx, cx, 0, e),
            TStmt::Break if brks == 0 && fin && cx.fin_brk => self.leave_via_finally(acx, cx, 1, None),
            TStmt::Continue if loops == 0 && fin && cx.fin_cont => self.leave_via_finally(acx, cx, 2, None),
            // The `break` of a transformed `switch`. At the end of a Wasm
            // loop body, the code after the `switch` is the next iteration.
            TStmt::Break if brks == 0 && cx.sw.is_some() => match cx.sw.as_deref() {
                Some(Tail::FallThrough(_)) => vec![TStmt::Continue],
                Some(t) => self.emit_tail(acx, t),
                None => unreachable!(),
            },
            TStmt::Return(e) => {
                let p = TExpr::new(TExprKind::Var(acx.p), Type::Struct(acx.info.sid), span);
                let v = match e {
                    Some(e) if acx.info.ty != Type::Void => e,
                    other => {
                        let mut out: Vec<TStmt> = other.into_iter().map(TStmt::Expr).collect();
                        out.extend(self.emit_tail(acx, &Tail::Resolve));
                        return out;
                    }
                };
                vec![void_stmt(TExprKind::Call(acx.info.resolve, vec![p, v]), span), TStmt::Return(None)]
            }
            TStmt::Break if brks == 0 && outside.is_some() => self.emit_tail(acx, &outside.unwrap().brk),
            TStmt::Continue if loops == 0 && outside.is_some() => self.emit_tail(acx, &Tail::Next(Box::new(outside.unwrap()))),
            TStmt::If(c, a, b) => {
                let a = self.sync_list(acx, a, cx, loops, brks);
                let b = self.sync_list(acx, b, cx, loops, brks);
                vec![TStmt::If(c, a, b)]
            }
            TStmt::Block(b) => vec![TStmt::Block(self.sync_list(acx, b, cx, loops, brks))],
            TStmt::Loop { id, cond, test_after, update, body } => {
                let body = self.sync_list(acx, body, cx, loops + 1, brks + 1);
                vec![TStmt::Loop { id, cond, test_after, update, body }]
            }
            TStmt::ForOf { id, var, arr, body } => {
                let body = self.sync_list(acx, body, cx, loops + 1, brks + 1);
                vec![TStmt::ForOf { id, var, arr, body }]
            }
            TStmt::Switch { disc, eq, cases } => {
                let cases = cases.into_iter().map(|(t, b)| (t, self.sync_list(acx, b, cx, loops, brks + 1))).collect();
                vec![TStmt::Switch { disc, eq, cases }]
            }
            TStmt::Try { body, catch, finally: Some(f) } if fin && cx.sync_fin.is_none() => {
                // A `try`/`finally` without `await` inside the `try` blocks
                // of an asynchronous `try`/`finally`.
                let flag = self.var_in(cx.owner, "$leave", Type::Bool, None);
                let used = Rc::new(std::cell::Cell::new(false));
                let icx = Ctx { sync_fin: Some((flag, used.clone())), ..cx.clone() };
                let body = self.sync_list(acx, body, &icx, loops, brks);
                let catch = catch.map(|(v, c)| (v, self.sync_list(acx, c, &icx, loops, brks)));
                // A `return` in the `finally` block itself leaves the
                // asynchronous `try`/`finally` directly.
                let mut f = self.sync_list(acx, f, cx, loops, brks);
                if !used.get() {
                    return vec![TStmt::Try { body, catch, finally: Some(f) }];
                }
                let fin = cx.fin.as_ref().expect("checked above").fin;
                let flag_r = TExpr::new(TExprKind::Var(flag), Type::Bool, span);
                f.push(TStmt::If(flag_r, vec![self.call_var(fin, span)], Vec::new()));
                vec![TStmt::Let(flag, Some(TExpr::new(TExprKind::Bool(false), Type::Bool, span))), TStmt::Try { body, catch, finally: Some(f) }]
            }
            TStmt::Try { body, catch, finally } => {
                let body = self.sync_list(acx, body, cx, loops, brks);
                let catch = catch.map(|(v, c)| (v, self.sync_list(acx, c, cx, loops, brks)));
                let finally = finally.map(|f| self.sync_list(acx, f, cx, loops, brks));
                vec![TStmt::Try { body, catch, finally }]
            }
            s => vec![s],
        }
    }

    /// A `return` (`kind` 0), `break` (1) or `continue` (2) that leaves a
    /// `try`/`finally` with an `await`: record it, then run the `finally`
    /// closure, which completes it.
    fn leave_via_finally(&mut self, acx: &Acx, cx: &Ctx, kind: usize, e: Option<TExpr>) -> Vec<TStmt> {
        let span = acx.span;
        let f = cx.fin.clone().expect("checked by the caller");
        let mut used = f.used.get();
        used[kind] = true;
        f.used.set(used);
        let mut out = Vec::new();
        if let Some(e) = e {
            out.push(match f.cv {
                Some(cv) => set_var(cv, e, span),
                None => TStmt::Expr(e),
            });
        }
        out.push(set_var(f.ck, int(kind as i32 + 2, span), span));
        match &cx.sync_fin {
            Some((flag, used)) => {
                used.set(true);
                out.push(set_var(*flag, TExpr::new(TExprKind::Bool(true), Type::Bool, span), span));
                out.push(TStmt::Return(None));
            }
            None => out.extend(self.emit_tail(acx, &Tail::Call(f.fin))),
        }
        out
    }

    fn sync_list(&mut self, acx: &Acx, stmts: Vec<TStmt>, cx: &Ctx, loops: u32, brks: u32) -> Vec<TStmt> {
        stmts.into_iter().flat_map(|s| self.sync_stmt(acx, s, cx, loops, brks)).collect()
    }

    /// `let x = await p; rest` (or `await p;`, `x = await p;`, `return
    /// await p;`): register a continuation on `p` that reads its value (or
    /// throws its error) and runs the rest, then return.
    fn await_stmt(&mut self, acx: &Acx, mut s: TStmt, rest: Vec<TStmt>, cx: &Ctx) -> Vec<TStmt> {
        let span = acx.span;
        let e = match &mut s {
            TStmt::Let(_, Some(e)) | TStmt::Expr(e) | TStmt::Return(Some(e)) => e,
            _ => unreachable!(),
        };
        let pe = top_await(e).expect("checked by the caller").clone();
        let Some(qinfo) = self.promise_of(&pe.ty).cloned() else {
            return vec![s];
        };
        let qty = Type::Struct(qinfo.sid);
        let q = self.var_in(cx.owner, "$q", qty.clone(), None);
        let q_r = TExpr::new(TExprKind::Var(q), qty, pe.span);
        let value = self.pfield(&q_r, qinfo.sid, VALUE, value_ty(&qinfo.ty));
        let value = if qinfo.ty == Type::Void { TExpr { ty: Type::Void, ..value } } else { value };
        replace_top_await(e, value);
        let bad = has_await_expr(e) || has_await_expr(&pe);
        if bad {
            self.err_help(
                code::ASYNC,
                pe.span,
                "only one `await` is allowed in this statement",
                "move each `await` into its own statement: `const v = await p;`",
            );
            return Vec::new();
        }
        // `await` of a `Promise<void>` as a statement has no value to use.
        let s = match s {
            TStmt::Expr(e) if e.ty == Type::Void && matches!(e.kind, TExprKind::Field(..)) => None,
            s => Some(s),
        };
        let rejected = TExpr::new(
            TExprKind::Cmp(CmpOp::Eq, EqKind::I32, bx(self.pfield(&q_r, qinfo.sid, STATE, Type::Int)), bx(int(2, span))),
            Type::Bool,
            span,
        );
        let err_ty = self.error_type();
        let error = TExpr::new(
            TExprKind::Coerce(Coercion::Retag, bx(self.pfield(&q_r, qinfo.sid, ERROR, err_ty.clone().nullable()))),
            err_ty,
            span,
        );
        let k = self.make_closure(acx, cx, "<await>", Vec::new(), |c, kcx| {
            let mut body = vec![TStmt::If(rejected, vec![TStmt::Throw(error)], Vec::new())];
            body.extend(c.block(acx, s.into_iter().chain(rest).collect(), kcx));
            body
        });
        vec![
            TStmt::Let(q, Some(pe)),
            void_stmt(TExprKind::Call(self.async_rt.as_ref().expect("made with the first promise").then, vec![self.as_base(q_r), k]), span),
            TStmt::Return(None),
        ]
    }

    /// An `if`, a block, a loop or a `try` that has an `await` inside.
    /// `cx.tail` is the code after the statement.
    fn compound(&mut self, acx: &Acx, s: TStmt, cx: &Ctx) -> Vec<TStmt> {
        let span = await_span(&s, acx.span);
        match s {
            TStmt::If(c, a, b) if !has_await_expr(&c) => {
                let a = self.block(acx, a, cx);
                let b = self.block(acx, b, cx);
                vec![TStmt::If(c, a, b)]
            }
            TStmt::Block(b) => self.block(acx, b, cx),
            TStmt::Loop { id, cond, test_after: false, update, body }
                if !cond.as_ref().is_some_and(has_await_expr) && !update.as_ref().is_some_and(has_await_expr) =>
            {
                self.async_loop(acx, id, cond, update, body, cx)
            }
            TStmt::ForOf { id, var, arr, body } if !has_await_expr(&arr) => {
                // `for (const x of a)` becomes an index loop, so a
                // continuation can resume it.
                let sp = arr.span;
                let arr_ty = arr.ty.clone();
                let elem_ty = self.prog.vars[var as usize].ty.clone();
                let av = self.var_in(cx.owner, "$arr", arr_ty.clone(), None);
                let iv = self.var_in(cx.owner, "$i", Type::Number, None);
                let a_r = TExpr::new(TExprKind::Var(av), arr_ty, sp);
                let i_r = TExpr::new(TExprKind::Var(iv), Type::Number, sp);
                let len = TExpr::new(TExprKind::Rt("arr_len", vec![a_r.clone()]), Type::Int, sp);
                let len = TExpr::new(TExprKind::Coerce(Coercion::I32ToNum, bx(len)), Type::Number, sp);
                let cond = TExpr::new(TExprKind::Cmp(CmpOp::Lt, EqKind::F64, bx(i_r.clone()), bx(len)), Type::Bool, sp);
                let elem = TExpr::new(TExprKind::Index(bx(a_r), bx(i_r.clone())), elem_ty, sp);
                let one = TExpr::new(TExprKind::Num(1.0), Type::Number, sp);
                let inc = TExpr::new(TExprKind::Num2(NumOp::Add, bx(i_r), bx(one)), Type::Number, sp);
                let mut new_body = vec![
                    TStmt::Let(var, Some(elem)),
                    TStmt::Expr(TExpr::new(TExprKind::Assign(Place::Var(iv), bx(inc)), Type::Number, sp)),
                ];
                new_body.extend(body);
                let mut out = vec![TStmt::Let(av, Some(arr)), TStmt::Let(iv, Some(TExpr::new(TExprKind::Num(0.0), Type::Number, sp)))];
                out.extend(self.async_loop(acx, id, Some(cond), None, new_body, cx));
                out
            }
            TStmt::Try { body, catch: Some((ev, cbody)), finally: None } => self.async_try(acx, body, ev, cbody, cx),
            TStmt::Try { body, catch, finally: Some(fbody) } => self.async_finally(acx, body, catch, fbody, cx),
            TStmt::Loop { id, cond: Some(c), test_after: true, update: None, body } => {
                // `do body while (c)` becomes
                // `first = true; while (true) { if (!first && !c) break; first = false; body }`,
                // written with nested `if`s so an `await` in `c` runs only
                // when the test runs.
                let sp = c.span;
                let first = self.var_in(cx.owner, "$first", Type::Bool, None);
                let first_r = TExpr::new(TExprKind::Var(first), Type::Bool, sp);
                let not = |e: TExpr| TExpr::new(TExprKind::Not(bx(e)), Type::Bool, sp);
                let test = TStmt::If(not(first_r), vec![TStmt::If(not(c), vec![TStmt::Break], Vec::new())], Vec::new());
                let mut new_body = vec![test, set_var(first, TExpr::new(TExprKind::Bool(false), Type::Bool, sp), sp)];
                new_body.extend(body);
                let mut out = vec![TStmt::Let(first, Some(TExpr::new(TExprKind::Bool(true), Type::Bool, sp)))];
                out.extend(self.compound(acx, TStmt::Loop { id, cond: None, test_after: false, update: None, body: new_body }, cx));
                out
            }
            TStmt::Switch { disc, eq, cases }
                if !has_await_expr(&disc) && cases.iter().all(|(t, _)| !t.as_ref().is_some_and(has_await_expr)) =>
            {
                self.async_switch(acx, disc, eq, cases, cx)
            }
            s => {
                let (msg, help) = match &s {
                    TStmt::Switch { .. } => ("`await` in a `case` value is not supported", "compute the value before the `switch`"),
                    _ => (
                        "`await` is not supported in this place",
                        "make the `await` its own statement: `await p;`, `const v = await p;`, `v = await p;` or `return await p;`",
                    ),
                };
                self.err_help(code::ASYNC, span, msg, help);
                vec![s]
            }
        }
    }

    /// Rewrites a statement whose own expressions have an `await` deep
    /// inside into statements where each `await` is the value of a `let`
    /// (or is in a nested statement). `None` when the awaits are only in
    /// nested statements.
    fn linearize_stmt(&mut self, cx: &Ctx, s: &TStmt) -> Option<Vec<TStmt>> {
        let mut pre = Vec::new();
        let s2 = match s.clone() {
            TStmt::Let(v, Some(e)) if has_await_expr(&e) => TStmt::Let(v, Some(self.lin(cx, e, &mut pre))),
            TStmt::Expr(e) if has_await_expr(&e) => TStmt::Expr(self.lin(cx, e, &mut pre)),
            TStmt::Return(Some(e)) if has_await_expr(&e) => TStmt::Return(Some(self.lin(cx, e, &mut pre))),
            TStmt::Throw(e) if has_await_expr(&e) => TStmt::Throw(self.lin(cx, e, &mut pre)),
            TStmt::If(c, a, b) if has_await_expr(&c) => TStmt::If(self.lin(cx, c, &mut pre), a, b),
            TStmt::ForOf { id, var, arr, body } if has_await_expr(&arr) => TStmt::ForOf { id, var, arr: self.lin(cx, arr, &mut pre), body },
            TStmt::Switch { disc, eq, cases } if has_await_expr(&disc) => TStmt::Switch { disc: self.lin(cx, disc, &mut pre), eq, cases },
            TStmt::Loop { id, cond: Some(c), test_after: false, update, body } if has_await_expr(&c) => {
                // `while (c) body` becomes `while (true) { …; if (!c) break; body }`.
                let mut head = Vec::new();
                let c = self.lin(cx, c, &mut head);
                let span = c.span;
                head.push(TStmt::If(TExpr::new(TExprKind::Not(bx(c)), Type::Bool, span), vec![TStmt::Break], Vec::new()));
                head.extend(body);
                TStmt::Loop { id, cond: None, test_after: false, update, body: head }
            }
            _ => return None,
        };
        pre.push(s2);
        Some(pre)
    }

    /// A temporary for the transform; `reown` gives it its real owner.
    fn tmp(&mut self, cx: &Ctx, ty: Type) -> VarId {
        self.var_in(cx.owner, "$a", ty, None)
    }

    /// Returns `e` without `await`: each `await` becomes a `let` in `pre`,
    /// and the parts that run before it are kept in variables first, so
    /// the order of evaluation does not change. `&&`, `||` and `?:` become
    /// `if` statements, so a part that does not run does not wait.
    fn lin(&mut self, cx: &Ctx, e: TExpr, pre: &mut Vec<TStmt>) -> TExpr {
        if !has_await_expr(&e) {
            return e;
        }
        let span = e.span;
        let ty = e.ty.clone();
        let is_and = matches!(e.kind, TExprKind::And(..));
        match e.kind {
            TExprKind::Await(inner) => {
                let inner = self.lin(cx, *inner, pre);
                let t = self.tmp(cx, ty.clone());
                pre.push(TStmt::Let(t, Some(TExpr::new(TExprKind::Await(bx(inner)), ty.clone(), span))));
                TExpr::new(TExprKind::Var(t), ty, span)
            }
            TExprKind::Block(stmts, v) => {
                pre.extend(stmts);
                self.lin(cx, *v, pre)
            }
            TExprKind::And(a, b) | TExprKind::Or(a, b) if has_await_expr(&b) => {
                let a = self.lin(cx, *a, pre);
                let t = self.tmp(cx, Type::Bool);
                pre.push(TStmt::Let(t, Some(a)));
                let tr = TExpr::new(TExprKind::Var(t), Type::Bool, span);
                let mut bp = Vec::new();
                let b = self.lin(cx, *b, &mut bp);
                bp.push(TStmt::Expr(TExpr::new(TExprKind::Assign(Place::Var(t), bx(b)), Type::Bool, span)));
                let test = if is_and { tr.clone() } else { TExpr::new(TExprKind::Not(bx(tr.clone())), Type::Bool, span) };
                pre.push(TStmt::If(test, bp, Vec::new()));
                tr
            }
            TExprKind::Cond(c, a, b) if has_await_expr(&a) || has_await_expr(&b) => {
                let c = self.lin(cx, *c, pre);
                let t = self.tmp(cx, ty.clone());
                pre.push(TStmt::Let(t, None));
                let branch = |this: &mut Self, x: TExpr| {
                    let mut bp = Vec::new();
                    let x = this.lin(cx, x, &mut bp);
                    bp.push(TStmt::Expr(TExpr::new(TExprKind::Assign(Place::Var(t), bx(x)), ty.clone(), span)));
                    bp
                };
                let ap = branch(self, *a);
                let bp = branch(self, *b);
                pre.push(TStmt::If(c, ap, bp));
                TExpr::new(TExprKind::Var(t), ty, span)
            }
            kind => {
                let mut e = TExpr::new(kind, ty, span);
                let mut kids = expr_children_mut(&mut e);
                let last = kids.iter().rposition(|k| has_await_expr(k)).unwrap_or(0);
                for (i, k) in kids.iter_mut().enumerate() {
                    if i > last {
                        break;
                    }
                    let taken = std::mem::replace(&mut **k, TExpr::new(TExprKind::Null, Type::Error, span));
                    let x = self.lin(cx, taken, pre);
                    let keep = i == last
                        || matches!(x.kind, TExprKind::Num(_) | TExprKind::Bool(_) | TExprKind::Str(_) | TExprKind::Null | TExprKind::Closure(_))
                        || x.ty == Type::Void;
                    **k = if keep {
                        x
                    } else {
                        let t = self.tmp(cx, x.ty.clone());
                        let xt = x.ty.clone();
                        let xs = x.span;
                        pre.push(TStmt::Let(t, Some(x)));
                        TExpr::new(TExprKind::Var(t), xt, xs)
                    };
                }
                drop(kids);
                e
            }
        }
    }

    /// A loop with an `await`: a loop closure that holds the Wasm loop.
    fn async_loop(&mut self, acx: &Acx, id: LoopId, cond: Option<TExpr>, update: Option<TExpr>, body: Vec<TStmt>, cx: &Ctx) -> Vec<TStmt> {
        let lv = self.var_in(cx.owner, "$loop", waiter_ty(), None);
        let l = self.make_closure(acx, cx, "<async loop>", Vec::new(), |c, lcx| {
            let lc = LoopCx { lv, update: update.clone(), brk: lcx.tail.clone(), in_tir: true };
            let bcx = Ctx {
                tail: Tail::FallThrough(Box::new(lc.clone())),
                lcx: Some(lc),
                sw: None,
                fin_brk: false,
                fin_cont: false,
                ..lcx.clone()
            };
            let body = c.block(acx, body, &bcx);
            let mut out = vec![TStmt::Loop { id, cond, test_after: false, update, body }];
            out.extend(c.emit_tail(acx, &lcx.tail));
            out
        });
        vec![TStmt::Let(lv, Some(l)), self.call_var(lv, acx.span), TStmt::Return(None)]
    }

    /// A `switch` with an `await`: `$m` gets the index of the case that
    /// matches (or of `default`), and each case body runs when its index is
    /// at least `$m`, so control falls through as in a `switch`. A `break`
    /// runs the code after the `switch` (`Ctx::sw`).
    fn async_switch(&mut self, acx: &Acx, disc: TExpr, eq: EqKind, cases: Vec<(Option<TExpr>, Vec<TStmt>)>, cx: &Ctx) -> Vec<TStmt> {
        let sp = disc.span;
        let dv = self.var_in(cx.owner, "$d", disc.ty.clone(), None);
        let d = TExpr::new(TExprKind::Var(dv), disc.ty.clone(), sp);
        let mv = self.var_in(cx.owner, "$m", Type::Int, None);
        let m = TExpr::new(TExprKind::Var(mv), Type::Int, sp);
        let default = cases.iter().position(|(t, _)| t.is_none()).unwrap_or(cases.len());
        let mut select = Vec::new();
        for (i, (t, _)) in cases.iter().enumerate().rev() {
            let Some(t) = t else { continue };
            let test = TExpr::new(TExprKind::Cmp(CmpOp::Eq, eq, bx(d.clone()), bx(t.clone())), Type::Bool, sp);
            select = vec![TStmt::If(test, vec![set_var(mv, int(i as i32, sp), sp)], select)];
        }
        let mut stmts = vec![TStmt::Let(dv, Some(disc)), TStmt::Let(mv, Some(int(default as i32, sp)))];
        stmts.extend(select);
        let mut bodies = Vec::new();
        for (i, (_, body)) in cases.into_iter().enumerate() {
            let run = TExpr::new(TExprKind::Cmp(CmpOp::Le, EqKind::I32, bx(m.clone()), bx(int(i as i32, sp))), Type::Bool, sp);
            bodies.push(TStmt::If(run, body, Vec::new()));
        }
        let scx = Ctx { sw: Some(Box::new(cx.tail.clone())), fin_brk: false, ..cx.clone() };
        stmts.extend(self.block(acx, bodies, &scx));
        stmts
    }

    /// A `try`/`finally` (with or without `catch`) with an `await`. The
    /// `finally` body becomes a closure. The end of the `try` and `catch`
    /// blocks calls it; so do an exception (`$ck` 1, the error in `$ce`),
    /// and a `return`, `break` or `continue` that leaves the blocks
    /// (`leave_via_finally`). After the `finally` body, a second closure
    /// completes what was pending.
    fn async_finally(&mut self, acx: &Acx, body: Vec<TStmt>, catch: Option<(VarId, Vec<TStmt>)>, fbody: Vec<TStmt>, cx: &Ctx) -> Vec<TStmt> {
        let span = acx.span;
        let err_ty = self.error_type();
        let nerr = err_ty.clone().nullable();
        let ck = self.var_in(cx.owner, "$ck", Type::Int, None);
        let ce = self.var_in(cx.owner, "$ce", nerr.clone(), None);
        let cv = (acx.info.ty != Type::Void).then(|| self.var_in(cx.owner, "$cv", acx.info.ty.clone(), None));
        let finv = self.var_in(cx.owner, "$finally", waiter_ty(), None);
        let fin = Rc::new(FinCx { ck, cv, fin: finv, used: std::cell::Cell::new([false; 3]) });

        // An exception in the `try` (without `catch`) or the `catch` block.
        let ev = self.var_in(cx.owner, "$e", err_ty.clone(), None);
        let (et, ne) = (err_ty.clone(), nerr.clone());
        let fh = self.make_closure(acx, cx, "<finally catch>", vec![ev], |c, _| {
            let e = TExpr::new(TExprKind::Var(ev), et, span);
            let mut b = vec![
                set_var(ce, TExpr::new(TExprKind::Coerce(Coercion::Retag, bx(e)), ne, span), span),
                set_var(ck, int(1, span), span),
            ];
            b.extend(c.emit_tail(acx, &Tail::Call(finv)));
            b
        });
        let fhv = self.var_in(cx.owner, "$fcatch", fh.ty.clone(), None);
        let mut icx = Ctx { tail: Tail::Call(finv), fin: Some(fin.clone()), fin_brk: true, fin_cont: true, sync_fin: None, ..cx.clone() };
        icx.handlers.push(fhv);
        let try_part = match catch {
            Some((ev, cbody)) => self.async_try(acx, body, ev, cbody, &icx),
            None => {
                let tbody = self.block(acx, body, &icx);
                let e2 = self.var_in(cx.owner, "$e", err_ty.clone(), None);
                let call = TExprKind::CallClosure(
                    bx(TExpr::new(TExprKind::Var(fhv), fh.ty.clone(), span)),
                    vec![TExpr::new(TExprKind::Var(e2), err_ty.clone(), span)],
                );
                vec![TStmt::Try { body: tbody, catch: Some((e2, vec![void_stmt(call, span), TStmt::Return(None)])), finally: None }]
            }
        };

        // After the `finally` body: complete the pending throw, `return`,
        // `break` or `continue`, or run the code after the statement.
        let used = fin.used.get();
        let (et, ne) = (err_ty.clone(), nerr.clone());
        let fd = self.make_closure(acx, cx, "<finally done>", Vec::new(), |c, dcx| {
            let ck_is = |n: i32| is(TExpr::new(TExprKind::Var(ck), Type::Int, span), n, span);
            let ce_r = TExpr::new(
                TExprKind::Coerce(Coercion::Retag, bx(TExpr::new(TExprKind::Var(ce), ne, span))),
                et,
                span,
            );
            let mut b = vec![TStmt::If(ck_is(1), vec![TStmt::Throw(ce_r)], Vec::new())];
            let pending = [
                TStmt::Return(cv.map(|v| TExpr::new(TExprKind::Var(v), acx.info.ty.clone(), span))),
                TStmt::Break,
                TStmt::Continue,
            ];
            for (i, s) in pending.into_iter().enumerate() {
                if used[i] {
                    let then = c.sync_stmt(acx, s, dcx, 0, 0);
                    b.push(TStmt::If(ck_is(i as i32 + 2), then, Vec::new()));
                }
            }
            b.extend(c.emit_tail(acx, &dcx.tail));
            b
        });
        let fdv = self.var_in(cx.owner, "$fdone", waiter_ty(), None);
        let fc = self.make_closure(acx, cx, "<finally>", Vec::new(), |c, fcx| {
            let fcx = Ctx { tail: Tail::Call(fdv), ..fcx.clone() };
            c.block(acx, fbody, &fcx)
        });
        let mut out = vec![
            TStmt::Let(ck, Some(int(0, span))),
            TStmt::Let(ce, Some(TExpr::new(TExprKind::Null, nerr.clone(), span))),
        ];
        if let Some(cv) = cv {
            out.push(TStmt::Let(cv, None));
        }
        out.extend([TStmt::Let(fdv, Some(fd)), TStmt::Let(finv, Some(fc)), TStmt::Let(fhv, Some(fh))]);
        out.extend(try_part);
        out
    }

    /// A `try`/`catch` with an `await`: the `catch` body becomes a handler
    /// closure, which every continuation inside the `try` calls.
    fn async_try(&mut self, acx: &Acx, body: Vec<TStmt>, ev: VarId, cbody: Vec<TStmt>, cx: &Ctx) -> Vec<TStmt> {
        let span = acx.span;
        let h = self.make_closure(acx, cx, "<async catch>", vec![ev], |c, hcx| c.block(acx, cbody, hcx));
        let hv = self.var_in(cx.owner, "$catch", h.ty.clone(), None);
        let mut bcx = cx.clone();
        bcx.handlers.push(hv);
        let tbody = self.block(acx, body, &bcx);
        let err_ty = self.error_type();
        let e2 = self.var_in(cx.owner, "$e", err_ty.clone(), None);
        let call = TExprKind::CallClosure(
            bx(TExpr::new(TExprKind::Var(hv), h.ty.clone(), span)),
            vec![TExpr::new(TExprKind::Var(e2), err_ty, span)],
        );
        vec![
            TStmt::Let(hv, Some(h)),
            TStmt::Try { body: tbody, catch: Some((e2, vec![void_stmt(call, span), TStmt::Return(None)])), finally: None },
        ]
    }
}

/// Gives every variable that `stmts` declares the owner `fid`, and as its
/// loop the innermost loop of `fid` around the declaration.
fn reown(prog: &mut Program, stmts: &[TStmt], fid: FuncId, lp: Option<LoopId>) {
    for s in stmts {
        let set = |prog: &mut Program, v: VarId, lp: Option<LoopId>| {
            let info = &mut prog.vars[v as usize];
            info.owner = fid;
            info.in_loop = lp;
        };
        let inner = match s {
            TStmt::Let(v, _) => {
                set(prog, *v, lp);
                lp
            }
            TStmt::ForOf { id, var, .. } => {
                set(prog, *var, Some(*id));
                Some(*id)
            }
            TStmt::Loop { id, .. } => Some(*id),
            TStmt::Try { catch: Some((v, _)), .. } => {
                set(prog, *v, lp);
                lp
            }
            _ => lp,
        };
        let (exprs, lists) = stmt_parts(s);
        // A loop's condition and update are outside its body.
        for e in exprs {
            reown_expr(prog, e, fid, lp);
        }
        for l in lists {
            reown(prog, l, fid, inner);
        }
    }
}

fn reown_expr(prog: &mut Program, e: &TExpr, fid: FuncId, lp: Option<LoopId>) {
    if let TExprKind::Block(stmts, _) = &e.kind {
        reown(prog, stmts, fid, lp);
    }
    for c in expr_children(e) {
        reown_expr(prog, c, fid, lp);
    }
}
