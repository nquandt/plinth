//! `String` methods that take a `RegExp` (PRODUCTION.md L4 step 3):
//! `match`, `matchAll`, `replace`, `replaceAll`, `split` and `search`.
//!
//! The work is in the core (`plinth-rt/src/regexp.rs`, the `regex_*`
//! functions of core 1.12). A call site is one core call, except
//! `matchAll` and `replace` with a function: these call a helper function
//! that the compiler makes one time per program (per callback arity), on
//! first use. An app that does not use them does not get the helpers.

use super::Checker;
use super::asyncfn::{bx, int, set_var};
use crate::ast::{Expr, ExprKind};
use crate::diag::{Span, code};
use crate::tir::*;
use crate::types::{Field, FuncType, StructId, Type};
use std::rc::Rc;

fn var(v: VarId, ty: &Type, span: Span) -> TExpr {
    TExpr::new(TExprKind::Var(v), ty.clone(), span)
}

/// `(string | null)[]`: the groups of one match.
fn groups_ty() -> Type {
    Type::Array(Box::new(Type::String.nullable()))
}

/// A replacement: a string, or a function and the number of parameters
/// it takes (0 to 3).
enum Replacement {
    Text(TExpr),
    Func(TExpr, usize),
}

impl Checker<'_> {
    /// Whether `re` has the `g` flag, when the compiler can know it: for a
    /// regular expression literal, for `new RegExp(p, "flags")` with
    /// literal flags, and for a `const` bound to one of these.
    pub(super) fn regex_static_global(&self, re: &TExpr) -> Option<bool> {
        match &re.kind {
            TExprKind::Block(stmts, _) => match stmts.first() {
                Some(TStmt::Let(_, Some(TExpr { kind: TExprKind::Rt("regex_compile", a), .. }))) => match a.get(1).map(|f| &f.kind) {
                    Some(TExprKind::Str(f)) => Some(f.contains('g')),
                    _ => None,
                },
                _ => None,
            },
            TExprKind::Var(v) => self.regex_globals.get(v).copied(),
            _ => None,
        }
    }

    /// The type of an `exec` result: `{ groups: (string | null)[]; index: number }`.
    pub(super) fn exec_result_sid(&mut self) -> StructId {
        match self.anon_struct(vec![
            Field { name: "groups".into(), ty: groups_ty(), optional: false },
            Field { name: "index".into(), ty: Type::Number, optional: false },
        ]) {
            Type::Struct(id) => id,
            _ => unreachable!("anon_struct always returns Type::Struct"),
        }
    }

    /// `re.exec(s)`: the exec result, or null.
    pub(super) fn regex_exec_expr(&mut self, re: TExpr, s: TExpr, span: Span) -> TExpr {
        let arr_ty = groups_ty().nullable();
        let av = self.temp(arr_ty.clone());
        let arr_read = var(av, &arr_ty, span);
        let call = TExpr::new(TExprKind::Rt("regex_exec", vec![re, s]), arr_ty.clone(), span);
        let is_null = TExpr::new(TExprKind::IsNull(bx(arr_read.clone())), Type::Bool, span);
        let sid = self.exec_result_sid();
        let groups_val = TExpr::new(TExprKind::Coerce(Coercion::Retag, bx(arr_read)), groups_ty(), span);
        let index_val = TExpr::new(TExprKind::Rt("regex_last_match_index", vec![]), Type::Number, span);
        let built = TExpr::new(TExprKind::StructLit(sid, vec![groups_val, index_val]), Type::Struct(sid), span);
        let result_ty = self.nullable(Type::Struct(sid), span);
        let built = self.coerce(built, &result_ty);
        let null_v = TExpr::new(TExprKind::Null, Type::Null, span);
        let null_v = self.coerce(null_v, &result_ty);
        let cond = TExpr::new(TExprKind::Cond(bx(is_null), bx(null_v), bx(built)), result_ty.clone(), span);
        TExpr::new(TExprKind::Block(vec![TStmt::Let(av, Some(call))], bx(cond)), result_ty, span)
    }

    /// `if (v === null) throw new Error(regex_last_error())`: the core
    /// returns null when a method needs the `g` flag and the pattern has
    /// not got it (JavaScript throws a `TypeError` there).
    fn throw_if_null(&mut self, v: TExpr, span: Span) -> TStmt {
        let is_null = TExpr::new(TExprKind::IsNull(bx(v)), Type::Bool, span);
        let msg = TExpr::new(TExprKind::Rt("regex_last_error", vec![]), Type::String, span);
        let err = self.new_error(msg);
        TStmt::If(is_null, vec![TStmt::Throw(err)], Vec::new())
    }

    /// The `string` methods that can take a `RegExp`. `Err` when the first
    /// argument is not a `RegExp` and the method has a string form: the
    /// caller then uses that form, with the receiver and the checked first
    /// argument that `Err` gives back.
    pub(super) fn string_regex_method(&mut self, o: TExpr, prop: &str, args: &[Expr], span: Span) -> Result<TExpr, (TExpr, Option<TExpr>)> {
        let Some(a0) = args.first() else {
            self.err(code::ARG_COUNT, span, "missing argument");
            return Ok(TExpr::new(TExprKind::Null, Type::Error, span));
        };
        let first = self.expr(a0, None);
        let re = match (&first.ty, prop) {
            (Type::RegExp, _) => first,
            (Type::Error, _) => {
                for a in &args[1..] {
                    self.expr(a, None);
                }
                return Ok(TExpr::new(TExprKind::Null, Type::Error, span));
            }
            // JavaScript makes a `RegExp` from a string argument.
            (_, "match" | "matchAll" | "search") => {
                let p = self.coerce(first, &Type::String);
                let flags = if prop == "matchAll" { "g" } else { "" };
                let f = TExpr::new(TExprKind::Str(flags.into()), Type::String, span);
                self.regex_new(p, f, span)
            }
            _ => return Err((o, Some(first))),
        };
        Ok(match prop {
            "search" => TExpr::new(TExprKind::Rt("regex_search", vec![o, re]), Type::Number, span),
            "split" => {
                let limit = self.num_arg(args, 1, 4294967295.0, span);
                TExpr::new(TExprKind::Rt("regex_split", vec![o, re, limit]), Type::Array(Box::new(Type::String)), span)
            }
            "match" => match self.regex_static_global(&re) {
                Some(true) => TExpr::new(TExprKind::Rt("regex_match", vec![o, re]), Type::Array(Box::new(Type::String)).nullable(), span),
                Some(false) => {
                    let t = self.temp(Type::String);
                    let e = self.regex_exec_expr(re, var(t, &Type::String, span), span);
                    let ty = e.ty.clone();
                    TExpr::new(TExprKind::Block(vec![TStmt::Let(t, Some(o))], bx(e)), ty, span)
                }
                None => {
                    self.err_help(
                        code::REGEX,
                        args[0].span,
                        "`match` needs to know if this RegExp has the `g` flag",
                        "use a regular expression literal or a `const` set to one, or use `re.exec(s)` or `s.matchAll(re)`",
                    );
                    TExpr::new(TExprKind::Null, Type::Error, span)
                }
            },
            "matchAll" => {
                let fid = self.match_all_helper();
                let sid = self.exec_result_sid();
                TExpr::new(TExprKind::Call(fid, vec![o, re]), Type::Array(Box::new(Type::Struct(sid))), span)
            }
            "replace" | "replaceAll" => {
                let all = prop == "replaceAll";
                // `replaceAll` needs `g`; check at run time unless it is known.
                let mode = if all && self.regex_static_global(&re) != Some(true) { 2 } else { 0 };
                match self.replacement(args, span) {
                    Replacement::Text(repl) => {
                        let call = TExpr::new(TExprKind::Rt("regex_replace", vec![o, re, repl, int(mode.min(1), span)]), Type::String, span);
                        if mode == 0 {
                            call
                        } else {
                            let t = self.temp(Type::String);
                            let guard = self.throw_if_null(var(t, &Type::String, span), span);
                            TExpr::new(TExprKind::Block(vec![TStmt::Let(t, Some(call)), guard], bx(var(t, &Type::String, span))), Type::String, span)
                        }
                    }
                    Replacement::Func(f, arity) => {
                        let fid = self.replace_fn_helper(arity);
                        TExpr::new(TExprKind::Call(fid, vec![o, re, f, int(mode, span)]), Type::String, span)
                    }
                }
            }
            _ => unreachable!("only the RegExp string methods come here"),
        })
    }

    /// The parameters that a replacement function gets: the match, the
    /// groups (as `exec` gives them) and the UTF-16 index of the match.
    fn replace_fn_params() -> [Type; 3] {
        [Type::String, groups_ty(), Type::Number]
    }

    fn replacement(&mut self, args: &[Expr], span: Span) -> Replacement {
        let Some(a) = args.get(1) else {
            self.err(code::ARG_COUNT, span, "missing argument");
            return Replacement::Text(TExpr::new(TExprKind::Str(String::new()), Type::String, span));
        };
        let params = Self::replace_fn_params();
        if matches!(a.kind, ExprKind::Func(_)) {
            let (f, n) = self.callback(a, &params, Some(Type::String));
            return Replacement::Func(f, n);
        }
        let te = self.expr(a, None);
        if matches!(te.ty, Type::Func(_)) {
            let (f, n) = self.callback_value(te, a.span, &params, Some(Type::String));
            return Replacement::Func(f, n);
        }
        Replacement::Text(self.coerce(te, &Type::String))
    }

    /// `$regexMatchAll(s, re)`: the exec results of all matches.
    fn match_all_helper(&mut self) -> FuncId {
        if let Some(f) = self.regex_helpers[0] {
            return f;
        }
        let span = Span::default();
        let sid = self.exec_result_sid();
        let out_ty = Type::Array(Box::new(Type::Struct(sid)));
        let (fid, ps) = self.helper_fn("$regexMatchAll", &[("s", Type::String), ("re", Type::RegExp)], out_ty.clone());
        let (sc, ix, mut body) = self.scan_prologue(fid, ps[0], ps[1], int(1, span));
        let ov = self.var_in(fid, "$out", out_ty.clone(), None);
        let out = var(ov, &out_ty, span);
        body.push(TStmt::Let(ov, Some(TExpr::new(TExprKind::ArrayLit(Vec::new()), out_ty.clone(), span))));
        let (iv, i) = self.counter(fid, &mut body);
        let groups = TExpr::new(TExprKind::Index(bx(sc.clone()), bx(i.clone())), groups_ty(), span);
        let index = TExpr::new(TExprKind::Index(bx(ix), bx(i)), Type::Number, span);
        let lit = TExpr::new(TExprKind::StructLit(sid, vec![groups, index]), Type::Struct(sid), span);
        let push = self.arr_push_discard(out.clone(), lit, span);
        body.push(self.count_loop(iv, &sc, vec![push]));
        body.push(TStmt::Return(Some(out)));
        self.prog.funcs[fid as usize].body = body;
        self.regex_helpers[0] = Some(fid);
        fid
    }

    /// `$regexReplaceFn<n>(s, re, f, mode)`: `replace` with a function `f`
    /// of `n` parameters. `mode` 2 is `replaceAll` without a known `g`.
    fn replace_fn_helper(&mut self, arity: usize) -> FuncId {
        let arity = arity.min(3);
        if let Some(f) = self.regex_helpers[1 + arity] {
            return f;
        }
        let span = Span::default();
        let params = Self::replace_fn_params();
        let f_ty = Type::Func(Rc::new(FuncType { params: params[..arity].to_vec(), required: arity, ret: Type::String }));
        let name = format!("$regexReplaceFn{arity}");
        let (fid, ps) = self.helper_fn(&name, &[("s", Type::String), ("re", Type::RegExp), ("f", f_ty.clone()), ("mode", Type::Int)], Type::String);
        let (sc, ix, mut body) = self.scan_prologue(fid, ps[0], ps[1], var(ps[3], &Type::Int, span));
        let str_arr = Type::Array(Box::new(Type::String));
        let rv = self.var_in(fid, "$reps", str_arr.clone(), None);
        let reps = var(rv, &str_arr, span);
        body.push(TStmt::Let(rv, Some(TExpr::new(TExprKind::ArrayLit(Vec::new()), str_arr, span))));
        let (iv, i) = self.counter(fid, &mut body);
        let groups = TExpr::new(TExprKind::Index(bx(sc.clone()), bx(i.clone())), groups_ty(), span);
        let whole = TExpr::new(TExprKind::Index(bx(groups.clone()), bx(TExpr::new(TExprKind::Num(0.0), Type::Number, span))), Type::String.nullable(), span);
        let whole = TExpr::new(TExprKind::Coerce(Coercion::Retag, bx(whole)), Type::String, span);
        let index = TExpr::new(TExprKind::Index(bx(ix.clone()), bx(i)), Type::Number, span);
        let mut call_args = vec![whole, groups, index];
        call_args.truncate(arity);
        let call = TExpr::new(TExprKind::CallClosure(bx(var(ps[2], &f_ty, span)), call_args), Type::String, span);
        let push = self.arr_push_discard(reps.clone(), call, span);
        body.push(self.count_loop(iv, &sc, vec![push]));
        let s = var(ps[0], &Type::String, span);
        body.push(TStmt::Return(Some(TExpr::new(TExprKind::Rt("regex_splice", vec![s, sc, ix, reps]), Type::String, span))));
        self.prog.funcs[fid as usize].body = body;
        self.regex_helpers[1 + arity] = Some(fid);
        fid
    }

    /// `const sc = regex_scan(s, re, mode); if (sc === null) throw ...;
    /// const ix = regex_scan_indices();`
    fn scan_prologue(&mut self, fid: FuncId, s: VarId, re: VarId, mode: TExpr) -> (TExpr, TExpr, Vec<TStmt>) {
        let span = Span::default();
        let sc_ty = Type::Array(Box::new(groups_ty()));
        let ix_ty = Type::Array(Box::new(Type::Number));
        let scv = self.var_in(fid, "$scan", sc_ty.clone(), None);
        let ixv = self.var_in(fid, "$index", ix_ty.clone(), None);
        let sc = var(scv, &sc_ty, span);
        let ix = var(ixv, &ix_ty, span);
        let scan = TExpr::new(TExprKind::Rt("regex_scan", vec![var(s, &Type::String, span), var(re, &Type::RegExp, span), mode]), sc_ty, span);
        let guard = self.throw_if_null(sc.clone(), span);
        let indices = TExpr::new(TExprKind::Rt("regex_scan_indices", vec![]), ix_ty, span);
        (sc, ix, vec![TStmt::Let(scv, Some(scan)), guard, TStmt::Let(ixv, Some(indices))])
    }

    /// `let i = 0`.
    fn counter(&mut self, fid: FuncId, body: &mut Vec<TStmt>) -> (VarId, TExpr) {
        let span = Span::default();
        let iv = self.var_in(fid, "$i", Type::Number, None);
        body.push(TStmt::Let(iv, Some(TExpr::new(TExprKind::Num(0.0), Type::Number, span))));
        (iv, var(iv, &Type::Number, span))
    }

    /// `for (; i < arr.length; i = i + 1) { body }`.
    fn count_loop(&mut self, iv: VarId, arr: &TExpr, body: Vec<TStmt>) -> TStmt {
        let span = Span::default();
        let i = var(iv, &Type::Number, span);
        let len = self.arr_len_of(arr.clone(), span);
        let cond = TExpr::new(TExprKind::Cmp(CmpOp::Lt, EqKind::F64, bx(i.clone()), bx(len)), Type::Bool, span);
        let one = TExpr::new(TExprKind::Num(1.0), Type::Number, span);
        let inc = TExpr::new(TExprKind::Num2(NumOp::Add, bx(i), bx(one)), Type::Number, span);
        let TStmt::Expr(update) = set_var(iv, inc, span) else { unreachable!("set_var makes an expression statement") };
        TStmt::Loop { id: self.prog.new_loop(), cond: Some(cond), test_after: false, update: Some(update), body }
    }
}
