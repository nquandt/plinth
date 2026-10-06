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
            let checked = self.stmt(s, false);
            // `if (x === null) return;` narrows `x` for the rest of the block.
            if let (StmtKind::If(_, _, None), Some(TStmt::If(cond, then, els))) = (&s.kind, checked.last())
                && els.is_empty()
                && always_exits(then)
            {
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

    fn with_narrowing(&mut self, narrow: Vec<(VarId, Type)>, s: &ast::Stmt) -> Vec<TStmt> {
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
                let c = self.expr(cond, Some(&Type::Bool));
                let c = self.truthy(c);
                let id = self.prog.new_loop();
                self.fx.loops.push(id);
                let b = self.scoped_block(body);
                self.fx.loops.pop();
                vec![TStmt::Loop { id, cond: Some(c), test_after: false, update: None, body: b }]
            }
            StmtKind::DoWhile(body, cond) => {
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
                let c = test.as_ref().map(|t| {
                    let c = self.expr(t, Some(&Type::Bool));
                    self.truthy(c)
                });
                let u = update.as_ref().map(|u| self.expr(u, None));
                let id = self.prog.new_loop();
                self.fx.loops.push(id);
                let b = self.scoped_block(body);
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
                    None => None,
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
                if te.is_none() && self.fx.ret.as_ref().is_some_and(|r| *r != Type::Void && !r.is_error()) {
                    self.err(code::MISSING_RETURN, s.span, "this function must return a value");
                }
                if te.is_none() && self.fx.ret.is_none() && self.fx.inferred.is_none() {
                    self.fx.inferred = Some(Type::Void);
                }
                vec![TStmt::Return(te)]
            }
            StmtKind::Break | StmtKind::Continue => {
                if self.fx.loops.is_empty() && !matches!(s.kind, StmtKind::Break) {
                    self.err(code::SYNTAX, s.span, "`continue` outside a loop");
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
                let eq = match &d_ty {
                    Type::Number => EqKind::F64,
                    Type::Bool | Type::Enum(_) => EqKind::I32,
                    Type::String => EqKind::Str,
                    Type::Nullable(inner) if **inner == Type::String => EqKind::NullStr,
                    Type::Nullable(inner) if **inner == Type::Number => EqKind::NullF64,
                    Type::Nullable(inner) if inner.repr() == crate::types::Repr::I32 => EqKind::NullI32,
                    Type::Error => EqKind::I32,
                    other => {
                        let msg = format!("`switch` works on numbers, strings and enums, not `{}`", self.show(other));
                        self.err(code::TYPE_MISMATCH, disc.span, msg);
                        EqKind::I32
                    }
                };
                let mut out_cases = Vec::new();
                self.push_scope();
                for (test, body) in cases {
                    let t = test.as_ref().map(|t| {
                        let te = self.expr(t, Some(&case_ty));
                        let target = match &d_ty {
                            Type::Nullable(inner) => (**inner).clone(),
                            t => t.clone(),
                        };
                        self.coerce(te, &target)
                    });
                    let b = self.block_stmts(body);
                    out_cases.push((t, b));
                }
                self.pop_scope();
                vec![TStmt::Switch { disc: d, eq, cases: out_cases }]
            }
            StmtKind::Throw(e) => {
                let te = self.expr(e, Some(&Type::String));
                let te = if te.ty.is_stringish() || te.ty.is_error() {
                    te
                } else {
                    self.err_help(code::TYPE_MISMATCH, e.span, "`throw` takes a string message", "write `throw \"message\"`");
                    te
                };
                vec![TStmt::Throw(te)]
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
            Pattern::Ident(name, span) => {
                let v = self.new_var(name, src.ty.clone(), mutable);
                self.define(name, *span, Binding::Var(v));
                out.push(TStmt::Let(v, Some(src)));
            }
            Pattern::Object(props, span) => {
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
            }
            Pattern::Array(elems, span) => {
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
                    out.extend(self.bind_part(sub, part, mutable));
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

    /// The variables that a condition narrows: `(when true, when false)`.
    pub(super) fn narrowing(&self, cond: &TExpr) -> (Vec<(VarId, Type)>, Vec<(VarId, Type)>) {
        match &cond.kind {
            TExprKind::IsNull(inner) => match self.narrowable(inner) {
                Some((v, t)) => (Vec::new(), vec![(v, t)]),
                None => (Vec::new(), Vec::new()),
            },
            TExprKind::Not(inner) => {
                let (t, f) = self.narrowing(inner);
                (f, t)
            }
            TExprKind::Coerce(Coercion::Truthy, inner) => match self.narrowable(inner) {
                Some((v, t)) => (vec![(v, t)], Vec::new()),
                None => (Vec::new(), Vec::new()),
            },
            TExprKind::And(a, b) => {
                let (mut ta, _) = self.narrowing(a);
                let (tb, _) = self.narrowing(b);
                ta.extend(tb);
                (ta, Vec::new())
            }
            TExprKind::Or(a, b) => {
                let (_, mut fa) = self.narrowing(a);
                let (_, fb) = self.narrowing(b);
                fa.extend(fb);
                (Vec::new(), fa)
            }
            // `x instanceof C` (SPEC.md §4.2 v1): narrows `x` to `C` in the
            // true branch. The false branch keeps `x`'s declared type (a
            // subclass is not the only thing it could still not be).
            TExprKind::InstanceOf(obj, sid) => match self.narrowable_var(obj) {
                Some(v) => (vec![(v, Type::Struct(*sid))], Vec::new()),
                None => (Vec::new(), Vec::new()),
            },
            // `typeof x === "..."` or a discriminant comparison, both
            // compiled to `UnionIs` (HANDOFF.md item 2).
            TExprKind::UnionIs(obj, idxs) => match (self.narrowable_var(obj), &obj.ty) {
                (Some(v), Type::Union(members)) => {
                    let picked: Vec<Type> = idxs.iter().map(|i| members[*i].clone()).collect();
                    let rest: Vec<Type> = members.iter().enumerate().filter(|(i, _)| !idxs.contains(i)).map(|(_, m)| m.clone()).collect();
                    (vec![(v, one_of(picked))], vec![(v, one_of(rest))])
                }
                _ => (Vec::new(), Vec::new()),
            },
            _ => (Vec::new(), Vec::new()),
        }
    }

    /// The variable a (possibly narrowed/retagged) expression reads, for
    /// narrowing. Unlike `narrowable`, any type is allowed; like it, only a
    /// `const` or a parameter (never reassigned inside the branch).
    fn narrowable_var(&self, e: &TExpr) -> Option<VarId> {
        let v = match &e.kind {
            TExprKind::Var(v) => *v,
            TExprKind::Coerce(Coercion::Retag, inner) => return self.narrowable_var(inner),
            _ => return None,
        };
        let info = &self.prog.vars[v as usize];
        let is_param = self.prog.funcs[info.owner as usize].params.contains(&v);
        if info.mutable && !is_param { None } else { Some(v) }
    }

    /// A read of a `const` or a parameter of a nullable type.
    fn narrowable(&self, e: &TExpr) -> Option<(VarId, Type)> {
        let TExprKind::Var(v) = e.kind else { return None };
        let info = &self.prog.vars[v as usize];
        let is_param = self.prog.funcs[info.owner as usize].params.contains(&v);
        if info.mutable && !is_param {
            return None;
        }
        match &info.ty {
            Type::Nullable(inner) => Some((v, (**inner).clone())),
            _ => None,
        }
    }

    pub(super) fn var_read(&mut self, v: VarId, span: Span) -> TExpr {
        let ty = self.prog.vars[v as usize].ty.clone();
        let read = TExpr::new(TExprKind::Var(v), ty.clone(), span);
        // `narrowed` only ever comes from `narrowable`, which only narrows a
        // variable whose declared type is `T | null`; `ty` here is that
        // `Nullable(_)`, so the box, if any, always needs unwrapping.
        match self.narrowed(v) {
            Some(n) if n.repr() == crate::types::Repr::F64 => TExpr::new(TExprKind::Coerce(Coercion::UnboxNum, Box::new(read)), n, span),
            Some(n) if n.repr() == crate::types::Repr::I32 => TExpr::new(TExprKind::Coerce(Coercion::UnboxI32, Box::new(read)), n, span),
            Some(n) => TExpr::new(TExprKind::Coerce(Coercion::Retag, Box::new(read)), n, span),
            None => read,
        }
    }
}
