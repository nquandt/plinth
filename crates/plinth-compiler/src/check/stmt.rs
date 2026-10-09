//! Statements, declarations, destructuring and null narrowing.

use super::{Binding, Checker, always_exits};
use crate::ast::{self, Pattern, StmtKind, VarKind};
use crate::diag::{Span, code};
use crate::tir::*;
use crate::types::Type;

/// The type of "one of these members": the lone member, or a union of
/// the rest, for the two branches of a `UnionIs` narrowing.
fn one_of(mut members: Vec<Type>) -> Type {
    match members.len() {
        0 => Type::Error,
        1 => members.pop().unwrap(),
        _ => Type::Union(members.into()),
    }
}

impl Checker<'_> {
    pub(super) fn block_stmts(&mut self, stmts: &[ast::Stmt]) -> Vec<TStmt> {
        let mut out = Vec::new();
        for s in stmts {
            // A `let` that a branch assigns (flow order) drops that `let`'s
            // narrowing in every live scope (`invalidate_var_narrowing`),
            // including scopes outside the branch. That is right when the
            // branch falls through, but wrong when it does not (`if (x ===
            // null) return;`, or, inside a loop, `if (x.skip) { x =
            // x.parent; continue; }`): the lines after the `if` are only
            // ever reached when the condition was false, so an assignment
            // made only on the true path never applies to them. Snapshot
            // the live scopes' narrowing first, so it can be restored in
            // that case.
            let snapshot: Vec<_> = self.fx.scopes.iter().map(|sc| sc.narrow.clone()).collect();
            let checked = self.stmt(s, false);
            // `if (x === null) return;` narrows `x` for the rest of the block.
            if let (StmtKind::If(_, _, None), Some(TStmt::If(cond, then, els))) = (&s.kind, checked.last())
                && els.is_empty()
                && (always_exits(then) || matches!(then.last(), Some(TStmt::Break | TStmt::Continue)))
            {
                for (sc, snap) in self.fx.scopes.iter_mut().zip(snapshot) {
                    sc.narrow = snap;
                }
                let (_, when_false) = self.narrowing(cond);
                if let Some(scope) = self.fx.scopes.last_mut() {
                    scope.narrow.extend(when_false);
                }
            }
            out.extend(checked);
        }
        out
    }

    fn scoped_block(&mut self, s: &ast::Stmt) -> Vec<TStmt> {
        self.push_scope();
        let out = match &s.kind {
            StmtKind::Block(b) => self.block_stmts(b),
            _ => self.stmt(s, false),
        };
        self.pop_scope();
        out
    }

    fn with_narrowing(&mut self, narrow: Vec<(super::NarrowKey, Type)>, s: &ast::Stmt) -> Vec<TStmt> {
        self.push_scope();
        self.fx.scopes.last_mut().unwrap().narrow.extend(narrow);
        let out = match &s.kind {
            StmtKind::Block(b) => self.block_stmts(b),
            _ => self.stmt(s, false),
        };
        self.pop_scope();
        out
    }

    pub(super) fn stmt(&mut self, s: &ast::Stmt, top_level: bool) -> Vec<TStmt> {
        match &s.kind {
            StmtKind::Empty => Vec::new(),
            StmtKind::Expr(e) => {
                let te = self.expr(e, None);
                vec![TStmt::Expr(te)]
            }
            StmtKind::Var(decls) => {
                let mut out = Vec::new();
                for d in decls {
                    out.extend(self.var_decl(d, top_level));
                }
                out
            }
            StmtKind::Func(f) => {
                // A nested function declaration is a `const` closure.
                let Some((name, span)) = &f.name else { return Vec::new() };
                let declared = f.params.iter().all(|p| p.ty.is_some()) && f.ret.is_some();
                let v = if declared {
                    // Declare first, so the function can call itself.
                    let mut ps = Vec::new();
                    for p in &f.params {
                        ps.push(self.resolve_type(p.ty.as_ref().unwrap()));
                    }
                    let ret = self.resolve_type(f.ret.as_ref().unwrap());
                    let required = ps.len();
                    let ty = Type::Func(std::rc::Rc::new(crate::types::FuncType { params: ps, required, ret }));
                    let v = self.new_var(name, ty, false);
                    self.define(name, *span, Binding::Var(v));
                    Some(v)
                } else {
                    None
                };
                let closure = self.closure(f, None, name);
                let v = match v {
                    Some(v) => v,
                    None => {
                        let v = self.new_var(name, closure.ty.clone(), false);
                        self.define(name, *span, Binding::Var(v));
                        v
                    }
                };
                let closure = self.coerce(closure, &self.prog.vars[v as usize].ty.clone());
                vec![TStmt::Let(v, Some(closure))]
            }
            StmtKind::Block(b) => {
                self.push_scope();
                let out = self.block_stmts(b);
                self.pop_scope();
                vec![TStmt::Block(out)]
            }
            StmtKind::If(cond, then, els) => {
                let c = self.expr(cond, Some(&Type::Bool));
                let c = self.truthy(c);
                let (when_true, when_false) = self.narrowing(&c);
                let t = self.with_narrowing(when_true, then);
                let e = match els {
                    Some(e) => self.with_narrowing(when_false, e),
                    None => Vec::new(),
                };
                vec![TStmt::If(c, t, e)]
            }
            StmtKind::While(cond, body) => {
                // Docs/GAPS.md "Narrowing in a loop that assigns the
                // narrowed variable": any narrowing from before the loop,
                // of a variable the loop itself assigns (its condition or
                // its body), only ever held for the first pass — drop it
                // before the condition narrows the body, so a stale
                // narrowing from before the loop is never read on a later
                // pass.
                let mut assigned = std::collections::HashSet::new();
                assigned_names_expr(cond, &mut assigned);
                assigned_names_stmt(body, &mut assigned);
                self.invalidate_named_var_narrowing(&assigned);
                let c = self.expr(cond, Some(&Type::Bool));
                let c = self.truthy(c);
                // The condition narrows at the start of every pass through
                // the body; an assignment inside the body drops it again
                // (flow order; `narrow_path`/`assign`).
                let (when_true, _) = self.narrowing(&c);
                let id = self.prog.new_loop();
                self.fx.loops.push(id);
                let b = self.with_narrowing(when_true, body);
                self.fx.loops.pop();
                vec![TStmt::Loop { id, cond: Some(c), test_after: false, update: None, body: b }]
            }
            StmtKind::DoWhile(body, cond) => {
                // Same reasoning as `while`: the body runs before the
                // condition is ever checked, so nothing narrows it from
                // the condition, but a variable the loop assigns still
                // cannot trust a narrowing from before the loop (the
                // second and later passes do not have it).
                let mut assigned = std::collections::HashSet::new();
                assigned_names_stmt(body, &mut assigned);
                assigned_names_expr(cond, &mut assigned);
                self.invalidate_named_var_narrowing(&assigned);
                let id = self.prog.new_loop();
                self.fx.loops.push(id);
                let b = self.scoped_block(body);
                self.fx.loops.pop();
                let c = self.expr(cond, Some(&Type::Bool));
                let c = self.truthy(c);
                vec![TStmt::Loop { id, cond: Some(c), test_after: true, update: None, body: b }]
            }
            StmtKind::For { init, test, update, body } => {
                // The loop variables belong to the enclosing scope (one
                // binding for all iterations; a documented deviation).
                self.push_scope();
                let mut out = Vec::new();
                if let Some(i) = init {
                    out.extend(self.stmt(i, false));
                }
                // Same as `while`: drop the narrowing of anything the loop
                // itself assigns (its test, its `update`, or its body)
                // before the test narrows the body — `init` runs once,
                // before the loop, like ordinary straight-line code, so it
                // is not part of this (its own assignment already took the
                // usual flow-order path above).
                let mut assigned = std::collections::HashSet::new();
                if let Some(t) = test {
                    assigned_names_expr(t, &mut assigned);
                }
                if let Some(u) = update {
                    assigned_names_expr(u, &mut assigned);
                }
                assigned_names_stmt(body, &mut assigned);
                self.invalidate_named_var_narrowing(&assigned);
                let c = test.as_ref().map(|t| {
                    let c = self.expr(t, Some(&Type::Bool));
                    self.truthy(c)
                });
                // Same as `while`: the test narrows at the start of the
                // body, and also while checking `update` — it runs at the
                // end of the same pass that the test let into the body, so
                // it sees the same narrowing (`for (; x !== null; x =
                // x.parent)`).
                let when_true = c.as_ref().map(|c| self.narrowing(c).0).unwrap_or_default();
                let id = self.prog.new_loop();
                self.fx.loops.push(id);
                self.push_scope();
                self.fx.scopes.last_mut().unwrap().narrow.extend(when_true);
                // Checked in the order they run within one pass: the body,
                // then `update` — so an assignment in the body (not in this
                // test) still drops the narrowing for `update` to see.
                let b = match &body.kind {
                    StmtKind::Block(b) => self.block_stmts(b),
                    _ => self.stmt(body, false),
                };
                let u = update.as_ref().map(|u| self.expr(u, None));
                self.pop_scope();
                self.fx.loops.pop();
                self.pop_scope();
                out.push(TStmt::Loop { id, cond: c, test_after: false, update: u, body: b });
                vec![TStmt::Block(out)]
            }
            StmtKind::ForOf { kind, pattern, iter, body } => {
                if let Some(out) = self.kv_for_of(*kind, pattern, iter, body) {
                    return out;
                }
                let arr = self.expr(iter, None);
                let elem = match &arr.ty {
                    Type::Array(t) => (**t).clone(),
                    Type::Error => Type::Error,
                    other => {
                        let msg = format!("`for…of` needs an array, a `Map` or a `Set`, not `{}`", self.show(other));
                        self.err(code::TYPE_MISMATCH, iter.span, msg);
                        Type::Error
                    }
                };
                // Same reasoning as `while`/`for`: a variable the body
                // assigns cannot trust a narrowing from before the loop.
                // `for…of` has no test to narrow the body from, so this
                // only ever drops narrowing here, it never re-adds any.
                let mut assigned = std::collections::HashSet::new();
                assigned_names_stmt(body, &mut assigned);
                self.invalidate_named_var_narrowing(&assigned);
                let id = self.prog.new_loop();
                self.fx.loops.push(id);
                self.push_scope();
                let mutable = *kind == VarKind::Let;
                let (var, mut prologue) = match pattern {
                    Pattern::Ident(name, span) => {
                        let v = self.new_var(name, elem, mutable);
                        self.define(name, *span, Binding::Var(v));
                        (v, Vec::new())
                    }
                    pat => {
                        let v = self.temp(elem.clone());
                        let src = TExpr::new(TExprKind::Var(v), elem, pat.span());
                        let stmts = self.destructure(pat, src, mutable);
                        (v, stmts)
                    }
                };
                let b = match &body.kind {
                    StmtKind::Block(b) => self.block_stmts(b),
                    _ => self.stmt(body, false),
                };
                prologue.extend(b);
                self.pop_scope();
                self.fx.loops.pop();
                vec![TStmt::ForOf { id, var, arr, body: prologue }]
            }
            StmtKind::Return(value) => {
                let te = match value {
                    None => match self.fx.ret.clone() {
                        // The function's declared return type already
                        // allows `null` (or is `null` itself): a bare
                        // `return;` returns that null value directly, so
                        // codegen never has to push "nothing" where the
                        // function's signature expects a value.
                        Some(r) if matches!(r, Type::Nullable(_) | Type::Null) => Some(TExpr::new(TExprKind::Null, r, s.span)),
                        Some(r) if r != Type::Void && !r.is_error() => {
                            self.err(code::MISSING_RETURN, s.span, "this function must return a value");
                            None
                        }
                        Some(_) => None,
                        // No declared return type: the type is inferred
                        // from the branches that do return a value. Record
                        // that a bare `return;` was seen; `finish_ret`
                        // (check/mod.rs) decides, once every branch is
                        // known, whether that makes the inferred type
                        // nullable (and rewrites this bare `return;` to
                        // `return null;` of it) or the function stays
                        // `void`.
                        None => {
                            self.fx.saw_void_return = true;
                            None
                        }
                    },
                    Some(e) => {
                        let expected = self.fx.ret.clone();
                        let te = self.expr(e, expected.as_ref());
                        Some(match &expected {
                            Some(Type::Void) => {
                                self.err(code::TYPE_MISMATCH, e.span, "this function returns `void`; remove the value");
                                te
                            }
                            Some(r) => self.coerce(te, r),
                            None => self.infer_return(te),
                        })
                    }
                };
                vec![TStmt::Return(te)]
            }
            StmtKind::Break | StmtKind::Continue => {
                if self.fx.loops.is_empty() {
                    if !matches!(s.kind, StmtKind::Break) {
                        self.err(code::SYNTAX, s.span, "`continue` outside a loop");
                    } else if self.fx.switches == 0 {
                        self.err(code::SYNTAX, s.span, "`break` outside a loop or a `switch`");
                    }
                }
                vec![if matches!(s.kind, StmtKind::Break) { TStmt::Break } else { TStmt::Continue }]
            }
            StmtKind::Switch(disc, cases) => {
                let d = self.expr(disc, None);
                // A null discriminant matches no case, so it goes to `default`.
                let (d_ty, case_ty) = match &d.ty {
                    Type::Nullable(inner) => (Type::Nullable(Box::new(self.widen((**inner).clone()))), (**inner).clone()),
                    t => (self.widen(t.clone()), t.clone()),
                };
                // `case null:` on a `T | null` discriminant: compare as two
                // `T | null` values (both null, or the same `T` value).
                let null_case = cases.iter().any(|(t, _)| matches!(t.as_ref().map(|t| &t.kind), Some(ast::ExprKind::Null)));
                let opt = if null_case { self.opt_eq_kind(&d_ty, &d_ty) } else { None };
                let eq = match &d_ty {
                    _ if let Some(k) = opt => k,
                    Type::Number => EqKind::F64,
                    Type::Bool | Type::Enum(_) => EqKind::I32,
                    Type::String => EqKind::Str,
                    Type::Nullable(inner) if **inner == Type::String => EqKind::NullStr,
                    Type::Nullable(inner) if **inner == Type::Number => EqKind::NullF64,
                    Type::Nullable(inner) if inner.repr() == crate::types::Repr::I32 => EqKind::NullI32,
                    t if self.is_string_union(t) => EqKind::UnionStr,
                    Type::Error => EqKind::I32,
                    other => {
                        let msg = format!("`switch` works on numbers, strings and enums, not `{}`", self.show(other));
                        self.err(code::TYPE_MISMATCH, disc.span, msg);
                        EqKind::I32
                    }
                };
                let mut out_cases = Vec::new();
                self.push_scope();
                self.fx.switches += 1;
                for (test, body) in cases {
                    let t = test.as_ref().map(|t| {
                        let te = self.expr(t, Some(&case_ty));
                        let target = match &d_ty {
                            Type::Nullable(_) if opt.is_some() => d_ty.clone(),
                            Type::Nullable(inner) => (**inner).clone(),
                            t => t.clone(),
                        };
                        self.coerce(te, &target)
                    });
                    let b = self.block_stmts(body);
                    out_cases.push((t, b));
                }
                self.fx.switches -= 1;
                self.pop_scope();
                vec![TStmt::Switch { disc: d, eq, cases: out_cases }]
            }
            StmtKind::Throw(e) => {
                let error_ty = self.error_type();
                let te = self.expr(e, Some(&error_ty));
                let te = if te.ty.is_stringish() {
                    // `throw "message"` throws `new Error("message")`.
                    self.new_error(te)
                } else if self.is_error_class(&te.ty) {
                    self.coerce(te, &error_ty)
                } else {
                    if !te.ty.is_error() {
                        let msg = format!("`throw` takes an `Error` or a string, not `{}`", self.show(&te.ty));
                        self.err_help(code::TYPE_MISMATCH, e.span, msg, "write `throw new Error(\"message\")`");
                    }
                    te.with_ty(Type::Error)
                };
                vec![TStmt::Throw(te)]
            }
            StmtKind::Try { block, catch, finally } => {
                self.push_scope();
                let body = self.block_stmts(block);
                self.pop_scope();
                let catch = catch.as_ref().map(|c| {
                    self.push_scope();
                    let ty = self.error_type();
                    // Not reassignable, so `instanceof` narrows it (a
                    // documented deviation: JS lets you assign to it).
                    let v = match &c.param {
                        Some((name, span)) => {
                            let v = self.new_var(name, ty, false);
                            self.define(name, *span, Binding::Var(v));
                            v
                        }
                        None => self.temp(ty),
                    };
                    let b = self.block_stmts(&c.body);
                    self.pop_scope();
                    (v, b)
                });
                let finally = finally.as_ref().map(|f| {
                    self.push_scope();
                    let b = self.block_stmts(f);
                    self.pop_scope();
                    b
                });
                vec![TStmt::Try { body, catch, finally }]
            }
        }
    }

    fn infer_return(&mut self, te: TExpr) -> TExpr {
        match self.fx.inferred.clone() {
            None => {
                self.fx.inferred = Some(self.widen(te.ty.clone()));
                let t = self.widen(te.ty.clone());
                self.coerce(te, &t)
            }
            Some(prev) => {
                // `return null` and `return x` make `T | null`.
                let merged = match (&prev, &te.ty) {
                    (Type::Null, t) | (t, Type::Null) if *t != Type::Null => {
                        let span = te.span;
                        Some(self.nullable(t.clone(), span))
                    }
                    (Type::StrLits(a), Type::StrLits(b)) => {
                        Some(Type::str_lits(a.iter().chain(b.iter()).cloned().collect()))
                    }
                    _ => None,
                };
                if let Some(m) = merged {
                    self.fx.inferred = Some(m.clone());
                    return self.coerce(te, &m);
                }
                self.coerce(te, &prev)
            }
        }
    }

    fn var_decl(&mut self, d: &ast::VarDecl, top_level: bool) -> Vec<TStmt> {
        let declared = d.ty.as_ref().map(|t| self.resolve_type(t));
        let mutable = d.kind == VarKind::Let;
        let fn_name = match &d.pattern {
            Pattern::Ident(n, _) => n.clone(),
            _ => "<closure>".into(),
        };
        let init = d.init.as_ref().map(|e| match (&e.kind, &declared) {
            (ast::ExprKind::Func(f), Some(Type::Func(ft))) => {
                let ft = ft.clone();
                let c = self.closure(f, Some(&ft), &fn_name);
                self.coerce(c, &Type::Func(ft))
            }
            (ast::ExprKind::Func(f), None) => self.closure(f, None, &fn_name),
            _ => {
                let te = self.expr(e, declared.as_ref());
                match &declared {
                    Some(t) => self.coerce(te, t),
                    None => te,
                }
            }
        });
        let ty = match (&declared, &init) {
            (Some(t), _) => t.clone(),
            (None, Some(i)) => {
                if i.ty == Type::Null {
                    self.err_help(code::CANNOT_INFER, d.span, "the type of `null` alone is unknown", "add a type: `let x: T | null = null`");
                    Type::Error
                } else if i.ty == Type::Void {
                    self.err(code::TYPE_MISMATCH, d.span, "this expression has no value");
                    Type::Error
                } else if mutable {
                    self.widen(i.ty.clone())
                } else {
                    i.ty.clone()
                }
            }
            (None, None) => {
                self.err_help(code::CANNOT_INFER, d.span, "a variable without a value needs a type", "add `: type`");
                Type::Error
            }
        };
        if d.kind == VarKind::Const && init.is_none() {
            self.err(code::SYNTAX, d.span, "a `const` needs a value");
        }
        let init = init.map(|i| if i.ty != ty { self.coerce(i, &ty) } else { i });
        match &d.pattern {
            Pattern::Ident(name, span) => {
                let v = self.new_var(name, ty, mutable);
                self.define(name, *span, Binding::Var(v));
                if !mutable && let Some(g) = init.as_ref().and_then(|i| self.regex_static_global(i)) {
                    self.regex_globals.insert(v, g);
                }
                if d.exported && top_level {
                    self.exports[self.module].insert(name.clone(), Binding::Var(v));
                }
                vec![TStmt::Let(v, init)]
            }
            pat => {
                let Some(init) = init else { return Vec::new() };
                let tmp = self.temp(ty.clone());
                let mut out = vec![TStmt::Let(tmp, Some(init))];
                let src = TExpr::new(TExprKind::Var(tmp), ty, pat.span());
                out.extend(self.destructure(pat, src, mutable));
                if d.exported {
                    self.err(code::UNSUPPORTED, d.span, "exported destructuring is not supported");
                }
                out
            }
        }
    }

    /// Binds the names of a pattern to parts of `src` (a variable read).
    pub(super) fn destructure(&mut self, pat: &Pattern, src: TExpr, mutable: bool) -> Vec<TStmt> {
        let mut out = Vec::new();
        match pat {
            Pattern::Default(inner, value, span) => {
                // `a = v`: `v` when the part is null (JS: undefined). On a
                // part that is never null the default never applies; it is
                // still checked against the part's type.
                let part = if matches!(src.ty, Type::Nullable(_)) {
                    self.coalesce(src, value, *span, None)
                } else {
                    let want = src.ty.clone();
                    let v = self.expr(value, Some(&want));
                    let _ = self.coerce(v, &want);
                    src
                };
                out.extend(self.bind_part(inner, part, mutable));
            }
            Pattern::Ident(name, span) => {
                let v = self.new_var(name, src.ty.clone(), mutable);
                self.define(name, *span, Binding::Var(v));
                out.push(TStmt::Let(v, Some(src)));
            }
            Pattern::Object(props, rest, span) => {
                let Type::Struct(sid) = src.ty else {
                    if !src.ty.is_error() {
                        let msg = format!("cannot destructure a value of type `{}`", self.show(&src.ty));
                        self.err(code::TYPE_MISMATCH, *span, msg);
                    }
                    return out;
                };
                for (key, sub) in props {
                    let Some((idx, field)) = self.prog.structs[sid as usize].field(key).map(|(i, f)| (i, f.clone())) else {
                        let msg = format!("`{}` has no field `{key}`", self.prog.structs[sid as usize].name);
                        self.err(code::NO_PROPERTY, sub.span(), msg);
                        continue;
                    };
                    let part =
                        TExpr::new(TExprKind::Field(Box::new(src.clone()), sid, idx as u32), field.ty.clone(), sub.span());
                    out.extend(self.bind_part(sub, part, mutable));
                }
                // `...rest`: a new object with the other fields, of an
                // object type with just those fields.
                if let Some(rest_pat) = rest {
                    if self.tuple_shape(sid).is_some() {
                        self.err(code::UNSUPPORTED, rest_pat.span(), "`...rest` in an object pattern needs an object, not a tuple");
                        return out;
                    }
                    let all = self.prog.structs[sid as usize].fields.clone();
                    let (mut fields, mut values) = (Vec::new(), Vec::new());
                    for (i, f) in all.iter().enumerate() {
                        if props.iter().any(|(k, _)| *k == f.name) {
                            continue;
                        }
                        values.push(TExpr::new(TExprKind::Field(Box::new(src.clone()), sid, i as u32), f.ty.clone(), rest_pat.span()));
                        fields.push(f.clone());
                    }
                    let ty = self.anon_struct(fields);
                    let Type::Struct(rest_sid) = ty else { unreachable!("anon_struct gives a struct") };
                    let part = TExpr::new(TExprKind::StructLit(rest_sid, values), ty, rest_pat.span());
                    out.extend(self.bind_part(rest_pat, part, mutable));
                }
            }
            Pattern::Array(elems, rest, span) => {
                // A tuple (`[k, v]` of `Map.entries()`): each name reads a field.
                if let Type::Struct(sid) = src.ty
                    && let Some(shape) = self.tuple_shape(sid)
                {
                    let n = shape.fixed.len();
                    // `...rest` of a tuple: the rest elements past the pattern's
                    // elements, as an array. It needs a rest element in the
                    // tuple type and at least the fixed elements before it.
                    if let Some(rest_pat) = rest {
                        match &shape.rest {
                            Some(r) if elems.len() >= n => {
                                let aty = Type::Array(Box::new(r.clone()));
                                let arr = TExpr::new(TExprKind::Field(Box::new(src.clone()), sid, shape.rest_field()), aty.clone(), *span);
                                let from = TExpr::new(TExprKind::Coerce(Coercion::NumToI32, Box::new(TExpr::new(TExprKind::Num((elems.len() - n) as f64), Type::Number, *span))), Type::Bool, *span);
                                let to = TExpr::new(TExprKind::Coerce(Coercion::NumToI32, Box::new(TExpr::new(TExprKind::Num(i32::MAX as f64), Type::Number, *span))), Type::Bool, *span);
                                let part = TExpr::new(TExprKind::Rt("arr_slice", vec![arr, from, to]), aty, rest_pat.span());
                                out.extend(self.bind_part(rest_pat, part, mutable));
                            }
                            _ => {
                                let msg = format!(
                                    "`...rest` needs a tuple with a rest element after the fixed ones; `{}` has none here",
                                    self.prog.structs[sid as usize].name
                                );
                                self.err(code::UNSUPPORTED, rest_pat.span(), msg);
                            }
                        }
                    }
                    if shape.rest.is_none() && elems.len() > n {
                        let msg = format!("`{}` has only {} element(s)", self.prog.structs[sid as usize].name, n);
                        self.err(code::TYPE_MISMATCH, *span, msg);
                        return out;
                    }
                    for (i, sub) in elems.iter().enumerate() {
                        let Some(sub) = sub else { continue };
                        let part = match &shape.rest {
                            Some(r) if i >= n => {
                                let aty = Type::Array(Box::new(r.clone()));
                                let arr = TExpr::new(TExprKind::Field(Box::new(src.clone()), sid, shape.rest_field()), aty, sub.span());
                                let k = TExpr::new(TExprKind::Num((i - n) as f64), Type::Number, sub.span());
                                TExpr::new(TExprKind::Index(Box::new(arr), Box::new(k)), r.clone(), sub.span())
                            }
                            _ => TExpr::new(TExprKind::Field(Box::new(src.clone()), sid, i as u32), shape.fixed[i].clone(), sub.span()),
                        };
                        out.extend(self.bind_part(sub, part, mutable));
                    }
                    return out;
                }
                let Type::Array(elem) = src.ty.clone() else {
                    if !src.ty.is_error() {
                        let msg = format!("cannot destructure a value of type `{}` as an array", self.show(&src.ty));
                        self.err(code::TYPE_MISMATCH, *span, msg);
                    }
                    return out;
                };
                for (i, sub) in elems.iter().enumerate() {
                    let Some(sub) = sub else { continue };
                    let idx = TExpr::new(TExprKind::Num(i as f64), Type::Number, sub.span());
                    let part = TExpr::new(TExprKind::Index(Box::new(src.clone()), Box::new(idx)), (*elem).clone(), sub.span());
                    if let Pattern::Default(inner, value, dspan) = sub {
                        // `[a = v] = xs`: JS gives `v` past the end (the
                        // element is undefined), where a read would trap.
                        let len = self.arr_len_of(src.clone(), *dspan);
                        let k = TExpr::new(TExprKind::Num(i as f64), Type::Number, *dspan);
                        let inside = TExpr::new(TExprKind::Cmp(CmpOp::Lt, EqKind::F64, Box::new(k), Box::new(len)), Type::Bool, *dspan);
                        if matches!(*elem, Type::Nullable(_)) {
                            // Past the end is null; the default then
                            // applies to it as to a null element.
                            let none = TExpr::new(TExprKind::Null, (*elem).clone(), *dspan);
                            let read = TExpr::new(TExprKind::Cond(Box::new(inside), Box::new(part), Box::new(none)), (*elem).clone(), *dspan);
                            out.extend(self.bind_part(sub, read, mutable));
                        } else {
                            let v = self.expr(value, Some(&elem));
                            let v = self.coerce(v, &elem);
                            let read = TExpr::new(TExprKind::Cond(Box::new(inside), Box::new(part), Box::new(v)), (*elem).clone(), *dspan);
                            out.extend(self.bind_part(inner, read, mutable));
                        }
                        continue;
                    }
                    out.extend(self.bind_part(sub, part, mutable));
                }
                // `...rest`: a new array of the elements after the pattern's elements.
                if let Some(rest_pat) = rest {
                    let num = |v: f64| TExpr::new(TExprKind::Coerce(Coercion::NumToI32, Box::new(TExpr::new(TExprKind::Num(v), Type::Number, *span))), Type::Bool, *span);
                    let part = TExpr::new(
                        TExprKind::Rt("arr_slice", vec![src.clone(), num(elems.len() as f64), num(i32::MAX as f64)]),
                        src.ty.clone(),
                        rest_pat.span(),
                    );
                    out.extend(self.bind_part(rest_pat, part, mutable));
                }
            }
        }
        out
    }

    fn bind_part(&mut self, sub: &Pattern, part: TExpr, mutable: bool) -> Vec<TStmt> {
        match sub {
            Pattern::Ident(..) => self.destructure(sub, part, mutable),
            nested => {
                let tmp = self.temp(part.ty.clone());
                let ty = part.ty.clone();
                let mut out = vec![TStmt::Let(tmp, Some(part))];
                out.extend(self.destructure(nested, TExpr::new(TExprKind::Var(tmp), ty, nested.span()), mutable));
                out
            }
        }
    }

    /// The variables/member paths that a condition narrows: `(when true,
    /// when false)`.
    pub(super) fn narrowing(&self, cond: &TExpr) -> super::Narrowing {
        (self.narrowing_when(cond, true), self.narrowing_when(cond, false))
    }

    /// One side of `narrowing`: what `cond` narrows when its value is
    /// `when`. It walks the condition with a stack, so a long `&&`/`||`
    /// chain does not recurse (docs/GAPS.md R3).
    fn narrowing_when(&self, cond: &TExpr, when: bool) -> Vec<(super::NarrowKey, Type)> {
        let mut out = Vec::new();
        let mut stack = vec![(cond, when)];
        while let Some((e, when)) = stack.pop() {
            match &e.kind {
                TExprKind::IsNull(inner) if !when => out.extend(self.narrowable(inner)),
                TExprKind::Not(inner) => stack.push((inner, !when)),
                TExprKind::Coerce(Coercion::Truthy, inner) if when => out.extend(self.narrowable(inner)),
                // `a && b` narrows when true (both sides), `a || b` when
                // false. Push `b` first: `a` comes first in the list.
                TExprKind::And(a, b) if when => stack.extend([(&**b, true), (&**a, true)]),
                TExprKind::Or(a, b) if !when => stack.extend([(&**b, false), (&**a, false)]),
                // `x instanceof C` (SPEC.md §4.2 v1): narrows `x` to `C` in
                // the true branch. The false branch keeps `x`'s declared type
                // (a subclass is not the only thing it could still not be).
                TExprKind::InstanceOf(obj, sid) if when => {
                    out.extend(self.narrow_path(obj).map(|k| (k, Type::Struct(*sid))));
                }
                // `typeof x === "..."` or a discriminant comparison, both
                // compiled to `UnionIs` (HANDOFF.md item 2).
                TExprKind::UnionIs(obj, idxs) => {
                    if let (Some(k), Type::Union(members)) = (self.narrow_path(obj), &obj.ty) {
                        let picked: Vec<Type> = if when {
                            idxs.iter().map(|i| members[*i].clone()).collect()
                        } else {
                            members.iter().enumerate().filter(|(i, _)| !idxs.contains(i)).map(|(_, m)| m.clone()).collect()
                        };
                        out.push((k, one_of(picked)));
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// The narrowing key a (possibly narrowed/retagged) expression reads,
    /// if it is one the checker is willing to narrow: a plain `const`/
    /// parameter variable, a plain `let` (see below), or a member path
    /// `a.b`/`a.b.c` rooted at a `const`/parameter, where every step is a
    /// direct struct field read (`check/expr.rs`'s `property`), never a
    /// method call or computed index. See `docs/language.md` ("Narrowing")
    /// for the exact rule and why it is capped at two field hops and
    /// dropped on assignment/calls. A loop: it stops at the third field of
    /// a long `a.b.c.d...` read.
    ///
    /// A plain `let` (no member path: `path` is still empty here) is
    /// narrowable too, but only while checking the function that declares
    /// it (`info.owner != self.fx.func` reads it through a closure
    /// instead, which is never narrowed — see below): `check/expr.rs`'s
    /// `assign` drops the narrowing of a `let` the moment it is assigned
    /// (flow order), so the checker never trusts a stale narrowing of it
    /// past an assignment, and at the head of a loop the checker drops the
    /// narrowing of anything the loop itself assigns before the loop's own
    /// condition narrows the body (`check/stmt.rs`'s `assigned_names_stmt`,
    /// used by `While`/`For`/`ForOf`/`DoWhile`). A `let` that some closure
    /// assigns, anywhere in it, is never narrowed, full stop
    /// (`self.fx.closure_assigned`, computed once before the function is
    /// checked by `closure_assigned_names_body`/`_stmt`/`_expr`): a
    /// closure can run at any call to it, not only when the checker
    /// reaches the assignment's own textual position, so flow-order
    /// tracking cannot see it coming.
    pub(super) fn narrow_path(&self, e: &TExpr) -> Option<super::NarrowKey> {
        let mut path = Vec::new();
        let mut e = e;
        loop {
            match &e.kind {
                TExprKind::Var(v) => {
                    let info = &self.prog.vars[*v as usize];
                    let is_param = self.prog.funcs[info.owner as usize].params.contains(v);
                    if info.mutable
                        && !is_param
                        && (!path.is_empty() || info.owner != self.fx.func || self.fx.closure_assigned.contains(&info.name))
                    {
                        return None;
                    }
                    path.reverse();
                    return Some((*v, path));
                }
                TExprKind::Coerce(Coercion::Retag, inner) => e = inner,
                TExprKind::Field(obj, _sid, idx) => {
                    if path.len() >= 2 {
                        return None;
                    }
                    path.push(*idx);
                    e = obj;
                }
                _ => return None,
            }
        }
    }

    /// Like `narrow_path`, but only for a path whose current type is
    /// `T | null` (the common case: `if (x !== null)`, `x ?? y`).
    fn narrowable(&self, e: &TExpr) -> Option<(super::NarrowKey, Type)> {
        let key = self.narrow_path(e)?;
        match &e.ty {
            Type::Nullable(inner) => Some((key, (**inner).clone())),
            _ => None,
        }
    }

    pub(super) fn var_read(&mut self, v: VarId, span: Span) -> TExpr {
        let ty = self.prog.vars[v as usize].ty.clone();
        let read = TExpr::new(TExprKind::Var(v), ty.clone(), span);
        self.apply_narrowing(read, (v, Vec::new()))
    }

    /// Wraps a freshly-built variable/member read in the coercion that
    /// matches whatever that path is currently narrowed to, or returns it
    /// unchanged if it is not narrowed.
    pub(super) fn apply_narrowing(&mut self, read: TExpr, key: super::NarrowKey) -> TExpr {
        let span = read.span;
        // `narrowed` only ever comes from `narrowable`, which only narrows a
        // path whose static type is `T | null`; `read.ty` here is that
        // `Nullable(_)`, so the box, if any, always needs unwrapping.
        match self.narrowed(&key) {
            Some(n) if n.repr() == crate::types::Repr::F64 => TExpr::new(TExprKind::Coerce(Coercion::UnboxNum, Box::new(read)), n, span),
            Some(n) if n.repr() == crate::types::Repr::I32 => TExpr::new(TExprKind::Coerce(Coercion::UnboxI32, Box::new(read)), n, span),
            Some(n) => TExpr::new(TExprKind::Coerce(Coercion::Retag, Box::new(read)), n, span),
            None => read,
        }
    }
}

/// Every plain name that an assignment (`x = …`, `x += …`, `x++`) targets
/// anywhere in `s`, at the AST level, before any type checking. Used at
/// the head of a loop (docs/GAPS.md "Narrowing in a loop that assigns the
/// narrowed variable"): a variable the loop (its condition, its `update`,
/// or its body) assigns cannot trust a narrowing from before the loop on
/// any pass but the first, so it is dropped before the loop's own
/// condition narrows the body. This does not look inside a nested
/// function/arrow (a closure): a `let` that a closure assigns is never
/// narrowed at all (`Checker::invalidate_let_narrowing`, called on every
/// call), so there is nothing here to drop for it.
pub(super) fn assigned_names_stmt(s: &ast::Stmt, out: &mut std::collections::HashSet<String>) {
    match &s.kind {
        StmtKind::Var(decls) => {
            for d in decls {
                if let Some(init) = &d.init {
                    assigned_names_expr(init, out);
                }
            }
        }
        StmtKind::Func(_) | StmtKind::Empty | StmtKind::Break | StmtKind::Continue => {}
        StmtKind::Expr(e) => assigned_names_expr(e, out),
        StmtKind::Block(b) => {
            for st in b {
                assigned_names_stmt(st, out);
            }
        }
        StmtKind::If(c, t, e) => {
            assigned_names_expr(c, out);
            assigned_names_stmt(t, out);
            if let Some(e) = e {
                assigned_names_stmt(e, out);
            }
        }
        StmtKind::While(c, b) => {
            assigned_names_expr(c, out);
            assigned_names_stmt(b, out);
        }
        StmtKind::DoWhile(b, c) => {
            assigned_names_stmt(b, out);
            assigned_names_expr(c, out);
        }
        StmtKind::For { init, test, update, body } => {
            if let Some(i) = init {
                assigned_names_stmt(i, out);
            }
            if let Some(t) = test {
                assigned_names_expr(t, out);
            }
            if let Some(u) = update {
                assigned_names_expr(u, out);
            }
            assigned_names_stmt(body, out);
        }
        StmtKind::ForOf { iter, body, .. } => {
            assigned_names_expr(iter, out);
            assigned_names_stmt(body, out);
        }
        StmtKind::Return(v) => {
            if let Some(e) = v {
                assigned_names_expr(e, out);
            }
        }
        StmtKind::Switch(d, cases) => {
            assigned_names_expr(d, out);
            for (t, b) in cases {
                if let Some(t) = t {
                    assigned_names_expr(t, out);
                }
                for st in b {
                    assigned_names_stmt(st, out);
                }
            }
        }
        StmtKind::Throw(e) => assigned_names_expr(e, out),
        StmtKind::Try { block, catch, finally } => {
            for st in block {
                assigned_names_stmt(st, out);
            }
            if let Some(c) = catch {
                for st in &c.body {
                    assigned_names_stmt(st, out);
                }
            }
            if let Some(f) = finally {
                for st in f {
                    assigned_names_stmt(st, out);
                }
            }
        }
    }
}

/// `assigned_names_stmt`'s expression half.
fn assigned_names_expr(e: &ast::Expr, out: &mut std::collections::HashSet<String>) {
    match &e.kind {
        ast::ExprKind::Assign { target, value, .. } => {
            if let ast::ExprKind::Ident(name) = &target.kind {
                out.insert(name.clone());
            }
            assigned_names_expr(target, out);
            assigned_names_expr(value, out);
        }
        ast::ExprKind::Update { target, .. } => {
            if let ast::ExprKind::Ident(name) = &target.kind {
                out.insert(name.clone());
            }
            assigned_names_expr(target, out);
        }
        // A closure is checked, and invalidated on every call, on its own
        // (see the doc comment above); its body is not scanned here.
        ast::ExprKind::Func(_) => {}
        ast::ExprKind::Template(_, exprs) => {
            for x in exprs {
                assigned_names_expr(x, out);
            }
        }
        ast::ExprKind::Array(items) => {
            for (_, x) in items {
                assigned_names_expr(x, out);
            }
        }
        ast::ExprKind::Object(props) => {
            for p in props {
                match p {
                    ast::ObjProp::Field(_, x, _) => assigned_names_expr(x, out),
                    ast::ObjProp::Spread(x) => assigned_names_expr(x, out),
                }
            }
        }
        ast::ExprKind::Member { obj, .. } => assigned_names_expr(obj, out),
        ast::ExprKind::Index { obj, index, .. } => {
            assigned_names_expr(obj, out);
            assigned_names_expr(index, out);
        }
        ast::ExprKind::Call { callee, args, .. } => {
            assigned_names_expr(callee, out);
            for a in args {
                assigned_names_expr(a, out);
            }
        }
        ast::ExprKind::Unary(_, x) | ast::ExprKind::Await(x) => assigned_names_expr(x, out),
        ast::ExprKind::Binary(_, l, r) | ast::ExprKind::Logical(_, l, r) => {
            assigned_names_expr(l, out);
            assigned_names_expr(r, out);
        }
        ast::ExprKind::Cond(c, t, f) => {
            assigned_names_expr(c, out);
            assigned_names_expr(t, out);
            assigned_names_expr(f, out);
        }
        ast::ExprKind::NewInstance(_, args) | ast::ExprKind::NewPromise(_, args) => {
            for a in args {
                assigned_names_expr(a, out);
            }
        }
        ast::ExprKind::InstanceOf(x, _, _) => assigned_names_expr(x, out),
        ast::ExprKind::As(x, _, _) => assigned_names_expr(x, out),
        ast::ExprKind::Jsx(j) => assigned_names_jsx(j, out),
        ast::ExprKind::Num(_)
        | ast::ExprKind::Str(_)
        | ast::ExprKind::Bool(_)
        | ast::ExprKind::Null
        | ast::ExprKind::Ident(_)
        | ast::ExprKind::New(..)
        | ast::ExprKind::Regex(..) => {}
    }
}

/// `assigned_names_expr`'s JSX half.
fn assigned_names_jsx(j: &ast::JsxElement, out: &mut std::collections::HashSet<String>) {
    for a in &j.attrs {
        if let Some(v) = &a.value {
            assigned_names_expr(v, out);
        }
    }
    for c in &j.children {
        match c {
            ast::JsxChild::Text(..) => {}
            ast::JsxChild::Expr(e) => assigned_names_expr(e, out),
            ast::JsxChild::Element(el) => assigned_names_jsx(el, out),
        }
    }
}

/// Every name that a function's own body assigns somewhere inside a
/// closure (an arrow, a function expression, or a nested function
/// declaration) nested in it, at any depth — not an assignment directly
/// in the function's own top-level flow, which `Checker::
/// invalidate_var_narrowing` already tracks in flow order as the checker
/// reaches it. A name in this set is never narrowed by `narrow_path`
/// (docs/GAPS.md "Narrowing in a loop that assigns the narrowed
/// variable", rule 1): a closure can run at any later call to it, not
/// only at the point where the assignment itself is checked, so flow
/// order cannot see it coming. Computed once, before a function's body
/// is checked, and kept in `FnCx::closure_assigned` for as long as that
/// function is the one being checked.
pub(super) fn closure_assigned_names_body(body: &ast::Body) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    match body {
        ast::Body::Block(stmts) => {
            for s in stmts {
                closure_assigned_names_stmt(s, false, &mut out);
            }
        }
        ast::Body::Expr(e) => closure_assigned_names_expr(e, false, &mut out),
    }
    out
}

/// `closure_assigned_names_body`'s statement half. `in_closure` is true
/// once the walk has entered a closure nested in the function whose body
/// this call started from; every assignment target found while it is
/// true is collected.
pub(super) fn closure_assigned_names_stmt(s: &ast::Stmt, in_closure: bool, out: &mut std::collections::HashSet<String>) {
    match &s.kind {
        StmtKind::Var(decls) => {
            for d in decls {
                if let Some(init) = &d.init {
                    closure_assigned_names_expr(init, in_closure, out);
                }
            }
        }
        // A nested function declaration is a closure: everything it
        // assigns, anywhere inside it, counts.
        StmtKind::Func(f) => {
            if let ast::Body::Block(stmts) = &f.body {
                for st in stmts {
                    closure_assigned_names_stmt(st, true, out);
                }
            } else if let ast::Body::Expr(e) = &f.body {
                closure_assigned_names_expr(e, true, out);
            }
        }
        StmtKind::Empty | StmtKind::Break | StmtKind::Continue => {}
        StmtKind::Expr(e) => closure_assigned_names_expr(e, in_closure, out),
        StmtKind::Block(b) => {
            for st in b {
                closure_assigned_names_stmt(st, in_closure, out);
            }
        }
        StmtKind::If(c, t, e) => {
            closure_assigned_names_expr(c, in_closure, out);
            closure_assigned_names_stmt(t, in_closure, out);
            if let Some(e) = e {
                closure_assigned_names_stmt(e, in_closure, out);
            }
        }
        StmtKind::While(c, b) => {
            closure_assigned_names_expr(c, in_closure, out);
            closure_assigned_names_stmt(b, in_closure, out);
        }
        StmtKind::DoWhile(b, c) => {
            closure_assigned_names_stmt(b, in_closure, out);
            closure_assigned_names_expr(c, in_closure, out);
        }
        StmtKind::For { init, test, update, body } => {
            if let Some(i) = init {
                closure_assigned_names_stmt(i, in_closure, out);
            }
            if let Some(t) = test {
                closure_assigned_names_expr(t, in_closure, out);
            }
            if let Some(u) = update {
                closure_assigned_names_expr(u, in_closure, out);
            }
            closure_assigned_names_stmt(body, in_closure, out);
        }
        StmtKind::ForOf { iter, body, .. } => {
            closure_assigned_names_expr(iter, in_closure, out);
            closure_assigned_names_stmt(body, in_closure, out);
        }
        StmtKind::Return(v) => {
            if let Some(e) = v {
                closure_assigned_names_expr(e, in_closure, out);
            }
        }
        StmtKind::Switch(d, cases) => {
            closure_assigned_names_expr(d, in_closure, out);
            for (t, b) in cases {
                if let Some(t) = t {
                    closure_assigned_names_expr(t, in_closure, out);
                }
                for st in b {
                    closure_assigned_names_stmt(st, in_closure, out);
                }
            }
        }
        StmtKind::Throw(e) => closure_assigned_names_expr(e, in_closure, out),
        StmtKind::Try { block, catch, finally } => {
            for st in block {
                closure_assigned_names_stmt(st, in_closure, out);
            }
            if let Some(c) = catch {
                for st in &c.body {
                    closure_assigned_names_stmt(st, in_closure, out);
                }
            }
            if let Some(f) = finally {
                for st in f {
                    closure_assigned_names_stmt(st, in_closure, out);
                }
            }
        }
    }
}

/// `closure_assigned_names_stmt`'s expression half.
fn closure_assigned_names_expr(e: &ast::Expr, in_closure: bool, out: &mut std::collections::HashSet<String>) {
    match &e.kind {
        ast::ExprKind::Assign { target, value, .. } => {
            if in_closure {
                if let ast::ExprKind::Ident(name) = &target.kind {
                    out.insert(name.clone());
                }
            }
            closure_assigned_names_expr(target, in_closure, out);
            closure_assigned_names_expr(value, in_closure, out);
        }
        ast::ExprKind::Update { target, .. } => {
            if in_closure {
                if let ast::ExprKind::Ident(name) = &target.kind {
                    out.insert(name.clone());
                }
            }
            closure_assigned_names_expr(target, in_closure, out);
        }
        // A closure nested in another closure: still a closure, so the
        // walk keeps `in_closure` true (it was already true to get here,
        // or becomes true now).
        ast::ExprKind::Func(f) => match &f.body {
            ast::Body::Block(stmts) => {
                for st in stmts {
                    closure_assigned_names_stmt(st, true, out);
                }
            }
            ast::Body::Expr(fe) => closure_assigned_names_expr(fe, true, out),
        },
        ast::ExprKind::Template(_, exprs) => {
            for x in exprs {
                closure_assigned_names_expr(x, in_closure, out);
            }
        }
        ast::ExprKind::Array(items) => {
            for (_, x) in items {
                closure_assigned_names_expr(x, in_closure, out);
            }
        }
        ast::ExprKind::Object(props) => {
            for p in props {
                match p {
                    ast::ObjProp::Field(_, x, _) => closure_assigned_names_expr(x, in_closure, out),
                    ast::ObjProp::Spread(x) => closure_assigned_names_expr(x, in_closure, out),
                }
            }
        }
        ast::ExprKind::Member { obj, .. } => closure_assigned_names_expr(obj, in_closure, out),
        ast::ExprKind::Index { obj, index, .. } => {
            closure_assigned_names_expr(obj, in_closure, out);
            closure_assigned_names_expr(index, in_closure, out);
        }
        ast::ExprKind::Call { callee, args, .. } => {
            closure_assigned_names_expr(callee, in_closure, out);
            for a in args {
                closure_assigned_names_expr(a, in_closure, out);
            }
        }
        ast::ExprKind::Unary(_, x) | ast::ExprKind::Await(x) => closure_assigned_names_expr(x, in_closure, out),
        ast::ExprKind::Binary(_, l, r) | ast::ExprKind::Logical(_, l, r) => {
            closure_assigned_names_expr(l, in_closure, out);
            closure_assigned_names_expr(r, in_closure, out);
        }
        ast::ExprKind::Cond(c, t, f) => {
            closure_assigned_names_expr(c, in_closure, out);
            closure_assigned_names_expr(t, in_closure, out);
            closure_assigned_names_expr(f, in_closure, out);
        }
        ast::ExprKind::NewInstance(_, args) | ast::ExprKind::NewPromise(_, args) => {
            for a in args {
                closure_assigned_names_expr(a, in_closure, out);
            }
        }
        ast::ExprKind::InstanceOf(x, _, _) => closure_assigned_names_expr(x, in_closure, out),
        ast::ExprKind::As(x, _, _) => closure_assigned_names_expr(x, in_closure, out),
        ast::ExprKind::Jsx(j) => closure_assigned_names_jsx(j, in_closure, out),
        ast::ExprKind::Num(_)
        | ast::ExprKind::Str(_)
        | ast::ExprKind::Bool(_)
        | ast::ExprKind::Null
        | ast::ExprKind::Ident(_)
        | ast::ExprKind::New(..)
        | ast::ExprKind::Regex(..) => {}
    }
}

/// `closure_assigned_names_expr`'s JSX half.
fn closure_assigned_names_jsx(j: &ast::JsxElement, in_closure: bool, out: &mut std::collections::HashSet<String>) {
    for a in &j.attrs {
        if let Some(v) = &a.value {
            closure_assigned_names_expr(v, in_closure, out);
        }
    }
    for c in &j.children {
        match c {
            ast::JsxChild::Text(..) => {}
            ast::JsxChild::Expr(e) => closure_assigned_names_expr(e, in_closure, out),
            ast::JsxChild::Element(el) => closure_assigned_names_jsx(el, in_closure, out),
        }
    }
}
