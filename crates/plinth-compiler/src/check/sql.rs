//! `sql` from `plinth:store` (core 1.13, `docs/SQL.md` §3): `exec`,
//! `query<T>`, `batch` and `migrate`. Each call returns a `Promise`. The
//! compiler writes the parameters (and the steps of `batch` and `migrate`)
//! as JSON text, calls the runtime function `sql_call`, and makes a wrapper
//! closure that reads the completion with `net_result_*`, as for
//! `plinth:files`. `query<T>` decodes the rows with the typed decoder of
//! `JSON.parse<T>`; a `boolean` field also reads 0 and 1 (`json_int_bools`).
//! A row that does not match `T` rejects the promise.

use super::{Binding, Checker};
use crate::ast::{self, Expr, ExprKind, ObjProp};
use crate::diag::{Diagnostic, Span, code};
use crate::tir::*;
use crate::types::{Field, Type};
use plinth_link::capabilities::STORE_SQL;

/// The `kind` of the WIT call `sql.run` (`wit/plinth/app.wit`).
const EXEC: i32 = 0;
const QUERY: i32 = 1;
const BATCH: i32 = 2;
const MIGRATE: i32 = 3;

/// What the wrapper closure makes from a successful completion.
enum Outcome {
    /// Decodes the JSON text into the value type; a failure rejects with
    /// this message.
    Decode(String),
    /// The int field of the completion (`migrate`: the new version).
    Int,
    /// Nothing (`batch`).
    Void,
}

impl Checker<'_> {
    pub(super) fn sql_call(
        &mut self,
        prop: &str,
        prop_span: Span,
        type_args: &[ast::TypeAnn],
        args: &[Expr],
        span: Span,
    ) -> TExpr {
        self.require_capability(STORE_SQL, span);
        let empty = |s: &str| TExpr::new(TExprKind::Str(s.into()), Type::String, span);
        // After an error in the call itself, the arguments are not checked
        // (as for `plinth:files`).
        let error = || TExpr::new(TExprKind::Null, Type::Error, span);
        if prop != "query" && !type_args.is_empty() {
            self.err(
                code::GENERIC_USER,
                span,
                format!("`sql.{prop}` takes no type argument"),
            );
        }
        let (kind, statement, params, value_ty, outcome) = match prop {
            "exec" | "query" => {
                let query = prop == "query";
                if args.is_empty() || args.len() > 2 {
                    self.err(
                        code::ARG_COUNT,
                        span,
                        format!(
                            "`sql.{prop}` takes a statement and an optional array of parameters"
                        ),
                    );
                    return error();
                }
                let value_ty = if query {
                    if type_args.len() != 1 {
                        self.err_help(
                            code::GENERIC_USER,
                            span,
                            "`sql.query` needs a type argument for the rows",
                            "write `sql.query<Note>(\"SELECT ...\")`",
                        );
                        return error();
                    }
                    let row = self.resolve_type(&type_args[0]);
                    let is_row = match &row {
                        Type::Struct(sid) => self.tuple_shape(*sid).is_none(),
                        Type::Map(k, _) => **k == Type::String,
                        Type::Error => true,
                        _ => false,
                    };
                    if !is_row {
                        let msg = format!(
                            "the rows of `sql.query` cannot be of type `{}`",
                            self.show(&row)
                        );
                        self.err_help(
                            code::TYPE_MISMATCH,
                            type_args[0].span(),
                            msg,
                            "use an object type with one field for each column",
                        );
                        return error();
                    }
                    Type::Array(Box::new(row))
                } else {
                    Type::Struct(self.sql_exec_result_struct())
                };
                self.sql_lint(&args[0]);
                let statement = self.expr_with(&args[0], &Type::String);
                let statement = self.coerce(statement, &Type::String);
                let params = match args.get(1) {
                    Some(p) => self.sql_params(p),
                    None => empty("[]"),
                };
                let message = if query {
                    format!(
                        "sql: a row does not match the type `{}`",
                        self.show(&value_ty)
                    )
                } else {
                    "sql: the host gave a result that is not valid".to_owned()
                };
                (
                    if query { QUERY } else { EXEC },
                    statement,
                    params,
                    value_ty,
                    Outcome::Decode(message),
                )
            }
            "batch" => {
                if args.len() != 1 {
                    self.err(
                        code::ARG_COUNT,
                        span,
                        "`sql.batch` takes one argument: an array of `{ sql, params }` steps",
                    );
                    return error();
                }
                let steps = self.sql_batch_steps(&args[0]);
                (BATCH, empty(""), steps, Type::Void, Outcome::Void)
            }
            "migrate" => {
                if args.len() != 1 {
                    self.err(
                        code::ARG_COUNT,
                        span,
                        "`sql.migrate` takes one argument: an array of statements",
                    );
                    return error();
                }
                let steps = self.sql_migrate_steps(&args[0]);
                (MIGRATE, empty(""), steps, Type::Number, Outcome::Int)
            }
            _ => {
                self.err_help(
                    code::NO_PROPERTY,
                    prop_span,
                    format!("`sql.{prop}` does not exist"),
                    "use `exec`, `query`, `batch` or `migrate`",
                );
                return error();
            }
        };
        self.sql_request(prop, kind, statement, params, value_ty, outcome, span)
    }

    /// The memoized `{ changes: number; lastInsertId: number }` struct that
    /// `sql.exec` resolves with.
    fn sql_exec_result_struct(&mut self) -> crate::types::StructId {
        let fields = vec![
            Field {
                name: "changes".into(),
                ty: Type::Number,
                optional: false,
            },
            Field {
                name: "lastInsertId".into(),
                ty: Type::Number,
                optional: false,
            },
        ];
        match self.anon_struct(fields) {
            Type::Struct(s) => s,
            _ => unreachable!(),
        }
    }

    /// The warning of `docs/SQL.md` §3: a statement that is not a string
    /// literal or a `const`.
    fn sql_lint(&mut self, e: &Expr) {
        let fixed = match &e.kind {
            ExprKind::Str(_) => true,
            ExprKind::Template(_, parts) => parts.is_empty(),
            ExprKind::Ident(name) => match self.lookup(name) {
                Some(Binding::Var(v)) => {
                    let info = &self.prog.vars[v as usize];
                    !info.mutable && !self.prog.funcs[info.owner as usize].params.contains(&v)
                }
                _ => false,
            },
            _ => false,
        };
        if !fixed {
            self.diags.push(
                Diagnostic::warning(code::SQL_DYNAMIC, e.span, "this SQL statement is not a string literal or a `const`")
                    .help("give values as `?` parameters; a statement built from strings can let SQL injection in"),
            );
        }
    }

    /// True if a value of type `ty` can be a SQL parameter (`number`,
    /// `string`, `boolean`, `null`, or one of them or `null`).
    fn sql_param_type(ty: &Type) -> bool {
        match ty {
            Type::Number
            | Type::Int
            | Type::Bool
            | Type::String
            | Type::StrLits(_)
            | Type::Null
            | Type::Error => true,
            Type::Nullable(inner) => Self::sql_param_type(inner),
            _ => false,
        }
    }

    fn sql_param_error(&mut self, ty: &Type, span: Span) {
        let msg = format!("a SQL parameter cannot be of type `{}`", self.show(ty));
        self.err_help(
            code::TYPE_MISMATCH,
            span,
            msg,
            "give a `number`, a `string`, a `boolean` or `null`",
        );
    }

    /// The JSON text of the parameters: an array literal (each element is
    /// checked alone, so the elements can be of different types) or an
    /// array of one parameter type.
    fn sql_params(&mut self, e: &Expr) -> TExpr {
        let span = e.span;
        if let ExprKind::Array(items) = &e.kind {
            let mut acc = TExpr::new(TExprKind::Str("[".into()), Type::String, span);
            for (i, (spread, item)) in items.iter().enumerate() {
                if *spread {
                    self.err_help(
                        code::UNSUPPORTED,
                        item.span,
                        "the parameters of a SQL call cannot use a spread",
                        "give each parameter",
                    );
                    continue;
                }
                let value = self.expr(item, None);
                if !Self::sql_param_type(&value.ty) {
                    let ty = value.ty.clone();
                    self.sql_param_error(&ty, item.span);
                    continue;
                }
                let text = self.json_stringify_value(value, item.span);
                if i > 0 {
                    acc = concat(
                        acc,
                        TExpr::new(TExprKind::Str(",".into()), Type::String, span),
                        span,
                    );
                }
                acc = concat(acc, text, span);
            }
            return concat(
                acc,
                TExpr::new(TExprKind::Str("]".into()), Type::String, span),
                span,
            );
        }
        let value = self.expr(e, None);
        match &value.ty {
            Type::Array(elem) if Self::sql_param_type(elem) => {
                self.json_stringify_value(value, span)
            }
            Type::Error => value,
            other => {
                let msg = format!(
                    "the parameters of a SQL call must be an array, not `{}`",
                    self.show(other)
                );
                self.err_help(code::TYPE_MISMATCH, span, msg, "write `[a, b]`; give a `number`, a `string`, a `boolean` or `null` for each `?`");
                TExpr::new(TExprKind::Str("[]".into()), Type::Error, span)
            }
        }
    }

    /// The JSON text of the steps of `batch`: `[{"sql": ..., "params": [...]}, ...]`.
    fn sql_batch_steps(&mut self, e: &Expr) -> TExpr {
        let span = e.span;
        if let ExprKind::Array(items) = &e.kind {
            let mut acc = TExpr::new(TExprKind::Str("[".into()), Type::String, span);
            for (i, (spread, item)) in items.iter().enumerate() {
                if *spread {
                    self.err_help(
                        code::UNSUPPORTED,
                        item.span,
                        "the steps of `sql.batch` cannot use a spread",
                        "give each step, or give one array value",
                    );
                    continue;
                }
                let ExprKind::Object(props) = &item.kind else {
                    self.err_help(
                        code::TYPE_MISMATCH,
                        item.span,
                        "a step of `sql.batch` must be an object",
                        "write `{ sql: \"...\", params: [...] }`",
                    );
                    continue;
                };
                let mut sql = None;
                let mut params = None;
                for p in props {
                    match p {
                        ObjProp::Field(name, value, _) if name == "sql" => {
                            self.sql_lint(value);
                            let s = self.expr_with(value, &Type::String);
                            let s = self.coerce(s, &Type::String);
                            sql = Some(TExpr::new(
                                TExprKind::Rt("json_quote_str", vec![s]),
                                Type::String,
                                value.span,
                            ));
                        }
                        ObjProp::Field(name, value, _) if name == "params" => {
                            params = Some(self.sql_params(value))
                        }
                        ObjProp::Field(name, _, fspan) => self.err_help(
                            code::NO_PROPERTY,
                            *fspan,
                            format!("a step of `sql.batch` has no `{name}` property"),
                            "use `sql` and `params`",
                        ),
                        ObjProp::Spread(s) => self.err(
                            code::UNSUPPORTED,
                            s.span,
                            "a step of `sql.batch` cannot use a spread",
                        ),
                    }
                }
                let Some(sql) = sql else {
                    self.err_help(
                        code::MISSING_FIELD,
                        item.span,
                        "a step of `sql.batch` needs `sql`",
                        "write `{ sql: \"...\", params: [...] }`",
                    );
                    continue;
                };
                let params = params
                    .unwrap_or_else(|| TExpr::new(TExprKind::Str("[]".into()), Type::String, span));
                if i > 0 {
                    acc = concat(
                        acc,
                        TExpr::new(TExprKind::Str(",".into()), Type::String, span),
                        span,
                    );
                }
                acc = concat(
                    acc,
                    TExpr::new(TExprKind::Str("{\"sql\":".into()), Type::String, span),
                    span,
                );
                acc = concat(acc, sql, span);
                acc = concat(
                    acc,
                    TExpr::new(TExprKind::Str(",\"params\":".into()), Type::String, span),
                    span,
                );
                acc = concat(acc, params, span);
                acc = concat(
                    acc,
                    TExpr::new(TExprKind::Str("}".into()), Type::String, span),
                    span,
                );
            }
            return concat(
                acc,
                TExpr::new(TExprKind::Str("]".into()), Type::String, span),
                span,
            );
        }
        // An array value: each element an object with `sql: string` and an
        // optional `params` array. The compiler cannot see the statements,
        // so there is no warning for them.
        let value = self.expr(e, None);
        let ok = match &value.ty {
            Type::Array(elem) => match &**elem {
                Type::Struct(sid) => {
                    let fields = &self.prog.structs[*sid as usize].fields;
                    fields
                        .iter()
                        .any(|f| f.name == "sql" && f.ty == Type::String)
                        && fields.iter().all(|f| match f.name.as_str() {
                            "sql" => true,
                            "params" => match &f.ty {
                                Type::Array(p) => Self::sql_param_type(p),
                                Type::Nullable(a) => {
                                    matches!(&**a, Type::Array(p) if Self::sql_param_type(p))
                                }
                                _ => false,
                            },
                            _ => false,
                        })
                }
                _ => false,
            },
            Type::Error => return value,
            _ => false,
        };
        if !ok {
            let msg = format!(
                "the steps of `sql.batch` cannot be of type `{}`",
                self.show(&value.ty)
            );
            self.err_help(code::TYPE_MISMATCH, span, msg, "give an array of `{ sql: string; params: T[] }`, where `T` is `number`, `string`, `boolean` or `null`");
            return TExpr::new(TExprKind::Str("[]".into()), Type::Error, span);
        }
        self.json_stringify_value(value, span)
    }

    /// The JSON text of the steps of `migrate`: an array of statements.
    fn sql_migrate_steps(&mut self, e: &Expr) -> TExpr {
        if let ExprKind::Array(items) = &e.kind {
            for (spread, item) in items {
                if !*spread {
                    self.sql_lint(item);
                }
            }
        }
        let strings = Type::Array(Box::new(Type::String));
        let value = self.expr_with(e, &strings);
        let value = self.coerce(value, &strings);
        self.json_stringify_value(value, e.span)
    }

    /// The `Promise` of one `sql_call`, and its wrapper closure:
    /// `e = error; if (e == null) { resolve(value) } else { reject(Error(e)) }`.
    #[allow(clippy::too_many_arguments)]
    fn sql_request(
        &mut self,
        name: &str,
        kind: i32,
        statement: TExpr,
        params: TExpr,
        value_ty: Type,
        outcome: Outcome,
        span: Span,
    ) -> TExpr {
        let fid = self.prog.new_func(FuncDef {
            name: format!("<sql_{name}_done>"),
            kind: FuncKind::Closure,
            params: Vec::new(),
            ret: Type::Void,
            body: Vec::new(),
            span,
        });
        let info = self.promise_info(&value_ty);
        let pty = Type::Struct(info.sid);
        let pv = self.temp(pty.clone());
        let p = TExpr::new(TExprKind::Var(pv), pty.clone(), span);
        let (_, reject_fn) = self.promise_then_reject();
        let resolve = |v: TExpr| {
            TStmt::Expr(TExpr::new(
                TExprKind::Call(info.resolve, vec![p.clone(), v]),
                Type::Void,
                span,
            ))
        };

        let e_ty = Type::String.nullable();
        let ev = self.temp(e_ty.clone());
        let e = TExpr::new(TExprKind::Var(ev), e_ty.clone(), span);
        let base = self.as_base(p.clone());
        let err = self.new_error(e.clone());
        let reject_e = TStmt::Expr(TExpr::new(
            TExprKind::Call(reject_fn, vec![base.clone(), err]),
            Type::Void,
            span,
        ));

        let success = match outcome {
            Outcome::Void => vec![resolve(TExpr::new(
                TExprKind::Bool(false),
                Type::Bool,
                span,
            ))],
            Outcome::Int => vec![resolve(TExpr::new(
                TExprKind::Rt("net_result_status", Vec::new()),
                Type::Number,
                span,
            ))],
            Outcome::Decode(message) => {
                let tv = self.temp(Type::String);
                let t = TExpr::new(TExprKind::Var(tv), Type::String, span);
                let rows = kind == QUERY;
                let saved = std::mem::replace(&mut self.json_int_bools, rows);
                let parsed = self.json_parse_value(t, &value_ty, span);
                self.json_int_bools = saved;
                let dv = self.temp(parsed.ty.clone());
                let d = TExpr::new(TExprKind::Var(dv), parsed.ty.clone(), span);
                let is_null = TExpr::new(TExprKind::IsNull(Box::new(d.clone())), Type::Bool, span);
                let msg = TExpr::new(TExprKind::Str(message), Type::String, span);
                let bad = self.new_error(msg);
                let reject_bad = TStmt::Expr(TExpr::new(
                    TExprKind::Call(reject_fn, vec![base, bad]),
                    Type::Void,
                    span,
                ));
                let value = TExpr::new(
                    TExprKind::Coerce(Coercion::Retag, Box::new(d)),
                    value_ty.clone(),
                    span,
                );
                vec![
                    TStmt::Let(
                        tv,
                        Some(TExpr::new(
                            TExprKind::Rt("net_result_text", Vec::new()),
                            Type::String,
                            span,
                        )),
                    ),
                    TStmt::Let(dv, Some(parsed)),
                    TStmt::If(is_null, vec![reject_bad], vec![resolve(value)]),
                ]
            }
        };
        let is_ok = TExpr::new(TExprKind::IsNull(Box::new(e)), Type::Bool, span);
        let body = vec![
            TStmt::Let(
                ev,
                Some(TExpr::new(
                    TExprKind::Rt("net_result_error", Vec::new()),
                    e_ty,
                    span,
                )),
            ),
            TStmt::If(is_ok, success, vec![reject_e]),
        ];
        super::asyncfn::reown(&mut self.prog, &body, fid, None);
        self.prog.funcs[fid as usize].body = body;
        let ft = crate::types::FuncType {
            params: Vec::new(),
            required: 0,
            ret: Type::Void,
        };
        let wrapper = TExpr::new(
            TExprKind::Closure(fid),
            Type::Func(std::rc::Rc::new(ft)),
            span,
        );
        let call = TExpr::new(
            TExprKind::HostCall(
                "sql_call",
                kind,
                Box::new(statement),
                Box::new(params),
                Box::new(wrapper),
            ),
            Type::Void,
            span,
        );
        TExpr::new(
            TExprKind::Block(
                vec![
                    TStmt::Let(pv, Some(self.new_promise(&info, span))),
                    TStmt::Expr(call),
                ],
                Box::new(p),
            ),
            pty,
            span,
        )
    }
}

fn concat(a: TExpr, b: TExpr, span: Span) -> TExpr {
    TExpr::new(
        TExprKind::Concat(Box::new(a), Box::new(b)),
        Type::String,
        span,
    )
}
