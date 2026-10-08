//! The compiler's own syntax tree for Plinth TS.
//!
//! `parse.rs` converts the oxc AST into this tree and rejects unsupported
//! syntax there. The rest of the compiler never sees oxc types.

use crate::diag::Span;

#[derive(Debug, Clone)]
pub struct Module {
    pub items: Vec<Item>,
}

#[derive(Debug, Clone)]
pub enum Item {
    Import(Import),
    Stmt(Stmt),
    Interface(Interface),
    TypeAlias(TypeAlias),
    Enum(EnumDecl),
    /// `export default <expr>`. A default-exported function is a `Stmt::Func`
    /// with `is_default` set.
    ExportDefault(Expr),
    /// `export { a, b as c }`.
    ExportNames(Vec<(String, String, Span)>),
    /// A basic class (SPEC.md §4.2, v0: no `extends`).
    Class(ClassDecl),
}

#[derive(Debug, Clone)]
pub struct ClassDecl {
    pub name: String,
    pub fields: Vec<ClassField>,
    /// `None` is an error (a class needs exactly one constructor).
    pub ctor: Option<CtorDecl>,
    pub methods: Vec<FuncDecl>,
    pub exported: bool,
    pub span: Span,
    /// `class B extends A` (SPEC.md §4.2, v1: single inheritance).
    pub extends: Option<(String, Span)>,
}

#[derive(Debug, Clone)]
pub struct ClassField {
    pub name: String,
    pub ty: TypeAnn,
    pub init: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct CtorDecl {
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Import {
    pub source: String,
    pub source_span: Span,
    pub default: Option<(String, Span)>,
    /// `(imported, local, span)`.
    pub names: Vec<(String, String, Span)>,
    pub type_only: bool,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Interface {
    pub name: String,
    pub fields: Vec<FieldAnn>,
    pub exported: bool,
    pub span: Span,
    /// Names of `<A, B, …>` type parameters (generic interfaces,
    /// monomorphized per distinct type arguments, like generic type
    /// aliases; HANDOFF.md item 1).
    pub type_params: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct FieldAnn {
    pub name: String,
    pub ty: TypeAnn,
    pub optional: bool,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TypeAlias {
    pub name: String,
    pub ty: TypeAnn,
    pub exported: bool,
    pub span: Span,
    /// Names of `<A, B, …>` type parameters (generic type aliases,
    /// monomorphized per distinct type arguments; HANDOFF.md item 1).
    pub type_params: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct EnumDecl {
    pub name: String,
    pub members: Vec<(String, Option<f64>, Span)>,
    pub exported: bool,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum TypeAnn {
    Number(Span),
    String(Span),
    Boolean(Span),
    Void(Span),
    Null(Span),
    StrLit(String, Span),
    NumLit(f64, Span),
    BoolLit(bool, Span),
    Named { name: String, args: Vec<TypeAnn>, span: Span },
    Array(Box<TypeAnn>, Span),
    Union(Vec<TypeAnn>, Span),
    Func { params: Vec<(String, TypeAnn, bool)>, ret: Box<TypeAnn>, span: Span },
    Object(Vec<FieldAnn>, Span),
    /// `[A, B?, ...C[]]`: a tuple. Optional elements come after the
    /// required ones; a rest element (its array type) can only be last.
    Tuple(Vec<(TypeAnn, TupleMark)>, Span),
}

/// The kind of a tuple element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TupleMark {
    Required,
    /// `T?`
    Optional,
    /// `...T[]`
    Rest,
}

impl TypeAnn {
    pub fn span(&self) -> Span {
        match self {
            TypeAnn::Number(s)
            | TypeAnn::String(s)
            | TypeAnn::Boolean(s)
            | TypeAnn::Void(s)
            | TypeAnn::Null(s)
            | TypeAnn::StrLit(_, s)
            | TypeAnn::NumLit(_, s)
            | TypeAnn::BoolLit(_, s)
            | TypeAnn::Array(_, s)
            | TypeAnn::Union(_, s)
            | TypeAnn::Object(_, s)
            | TypeAnn::Tuple(_, s) => *s,
            TypeAnn::Named { span, .. } | TypeAnn::Func { span, .. } => *span,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarKind {
    Let,
    Const,
}

#[derive(Debug, Clone)]
pub enum Pattern {
    Ident(String, Span),
    /// `{ a, b: c, ...rest }`: `(key, pattern)` and the rest.
    Object(Vec<(String, Pattern)>, Option<Box<Pattern>>, Span),
    /// `[a, , b, ...rest]`: the elements (`None` for a hole) and the rest.
    Array(Vec<Option<Pattern>>, Option<Box<Pattern>>, Span),
    /// `a = 1` inside an object or array pattern: the default value is
    /// used when the part is `null` (`undefined` is the same value).
    Default(Box<Pattern>, Box<Expr>, Span),
}

impl Pattern {
    pub fn span(&self) -> Span {
        match self {
            Pattern::Ident(_, s) | Pattern::Object(_, _, s) | Pattern::Array(_, _, s) | Pattern::Default(_, _, s) => *s,
        }
    }
}

#[derive(Debug, Clone)]
pub struct VarDecl {
    pub kind: VarKind,
    pub pattern: Pattern,
    pub ty: Option<TypeAnn>,
    pub init: Option<Expr>,
    pub exported: bool,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub pattern: Pattern,
    pub ty: Option<TypeAnn>,
    pub default: Option<Expr>,
    pub optional: bool,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum Body {
    Expr(Box<Expr>),
    Block(Vec<Stmt>),
}

#[derive(Debug, Clone)]
pub struct FuncDecl {
    pub name: Option<(String, Span)>,
    pub params: Vec<Param>,
    pub ret: Option<TypeAnn>,
    pub body: Body,
    pub exported: bool,
    pub is_default: bool,
    pub span: Span,
    /// Names of `<T, U, …>` type parameters (generic functions, monomorphized
    /// per call site; HANDOFF.md item 3). Empty for an ordinary function.
    pub type_params: Vec<String>,
    /// `async function` / `async () => …` (SPEC.md §4.5).
    pub is_async: bool,
}

#[derive(Debug, Clone)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum StmtKind {
    Var(Vec<VarDecl>),
    Func(FuncDecl),
    Expr(Expr),
    Block(Vec<Stmt>),
    If(Expr, Box<Stmt>, Option<Box<Stmt>>),
    While(Expr, Box<Stmt>),
    DoWhile(Box<Stmt>, Expr),
    For { init: Option<Box<Stmt>>, test: Option<Expr>, update: Option<Expr>, body: Box<Stmt> },
    ForOf { kind: VarKind, pattern: Pattern, iter: Expr, body: Box<Stmt> },
    Return(Option<Expr>),
    Break,
    Continue,
    Switch(Expr, Vec<(Option<Expr>, Vec<Stmt>)>),
    Throw(Expr),
    /// `try { … } catch (e) { … } finally { … }` (SPEC.md §5.6). At least
    /// one of `catch` and `finally` is present.
    Try { block: Vec<Stmt>, catch: Option<Catch>, finally: Option<Vec<Stmt>> },
    Empty,
}

/// The `catch` clause of a `try` statement. `param` is `None` for
/// `catch { … }` (no binding).
#[derive(Debug, Clone)]
pub struct Catch {
    pub param: Option<(String, Span)>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicOp {
    And,
    Or,
    Nullish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Plus,
    Not,
    /// Only valid directly in `typeof x === "..."` (union narrowing).
    Typeof,
}

#[derive(Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    Num(f64),
    Str(String),
    Bool(bool),
    Null,
    Template(Vec<String>, Vec<Expr>),
    Ident(String),
    Array(Vec<(bool, Expr)>),
    Object(Vec<ObjProp>),
    Member { obj: Box<Expr>, prop: String, prop_span: Span, optional: bool },
    Index { obj: Box<Expr>, index: Box<Expr>, optional: bool },
    Call { callee: Box<Expr>, type_args: Vec<TypeAnn>, args: Vec<Expr>, optional: bool },
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Logical(LogicOp, Box<Expr>, Box<Expr>),
    /// `target op= value`; `op` is `None` for plain `=`.
    Assign { op: Option<BinOp>, target: Box<Expr>, value: Box<Expr> },
    /// `++x`, `x--` and so on.
    Update { inc: bool, prefix: bool, target: Box<Expr> },
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Func(Box<FuncDecl>),
    Jsx(Box<JsxElement>),
    /// `new Map<K, V>()` or `new Set<T>()` (HANDOFF.md item 5). No other
    /// built-in `new` expression is supported.
    New(String, Vec<TypeAnn>),
    /// `new C(args)`: construction of a user class (SPEC.md §4.2).
    NewInstance(String, Vec<Expr>),
    /// `new Promise<T>(executor)` (SPEC.md §4.5).
    NewPromise(Vec<TypeAnn>, Vec<Expr>),
    /// `x instanceof C` (SPEC.md §4.2 v1): `(value, class name, name span)`.
    InstanceOf(Box<Expr>, String, Span),
    /// `expr as T` (dogfooding gap #6): only a safe, checked narrowing is
    /// allowed — `string`/a wider literal union to a narrower literal
    /// union, or a discriminated union to one of its members. The checker
    /// (`check/expr.rs::as_cast`) decides which, and rejects everything
    /// else with `PL2006`.
    As(Box<Expr>, TypeAnn, Span),
    /// `await e` (SPEC.md §4.5), only inside an `async` function.
    Await(Box<Expr>),
    /// `/pattern/flags` (PRODUCTION.md L4, step 2). The pattern and flags
    /// are validated at parse time (`parse.rs`), the same way a template
    /// literal or number literal is checked eagerly.
    Regex(String, String),
}

#[derive(Debug, Clone)]
pub enum ObjProp {
    Field(String, Expr, Span),
    Spread(Expr),
}

#[derive(Debug, Clone)]
pub struct JsxElement {
    pub name: String,
    pub name_span: Span,
    pub attrs: Vec<JsxAttr>,
    pub children: Vec<JsxChild>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct JsxAttr {
    pub name: String,
    /// `None` for a bare attribute (`<X disabled />`), which means `true`.
    pub value: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum JsxChild {
    Text(String, Span),
    Expr(Expr),
    Element(JsxElement),
}

/// Copies a left-deep chain (`a + b + c`, `x.f().g()`) with a loop, not
/// recursion (docs/GAPS.md R3).
impl Clone for Expr {
    fn clone(&self) -> Self {
        let mut spine = Vec::new();
        let mut cur = self;
        while let Some(left) = chain_left(&cur.kind) {
            spine.push(cur);
            cur = left;
        }
        let mut acc = Expr { kind: cur.kind.clone(), span: cur.span };
        while let Some(node) = spine.pop() {
            let l = Box::new(acc);
            let kind = match &node.kind {
                ExprKind::Binary(op, _, r) => ExprKind::Binary(*op, l, r.clone()),
                ExprKind::Logical(op, _, r) => ExprKind::Logical(*op, l, r.clone()),
                ExprKind::Member { prop, prop_span, optional, .. } => {
                    ExprKind::Member { obj: l, prop: prop.clone(), prop_span: *prop_span, optional: *optional }
                }
                ExprKind::Index { index, optional, .. } => ExprKind::Index { obj: l, index: index.clone(), optional: *optional },
                ExprKind::Call { type_args, args, optional, .. } => {
                    ExprKind::Call { callee: l, type_args: type_args.clone(), args: args.clone(), optional: *optional }
                }
                _ => unreachable!("`chain_left` gives only these"),
            };
            acc = Expr { kind, span: node.span };
        }
        acc
    }
}

/// The left side of a chain link (`Clone` and `Drop` of `Expr`).
fn chain_left(kind: &ExprKind) -> Option<&Expr> {
    match kind {
        ExprKind::Binary(_, l, _) | ExprKind::Logical(_, l, _) => Some(l),
        ExprKind::Member { obj, .. } | ExprKind::Index { obj, .. } => Some(obj),
        ExprKind::Call { callee, .. } => Some(callee),
        _ => None,
    }
}

/// Drops a left-deep chain (`a + b + c`, `x.f().g()`) with a loop. The
/// default drop recurses once per link, and a chain has no length limit
/// (docs/GAPS.md R3).
impl Drop for Expr {
    fn drop(&mut self) {
        let mut next = take_chain_left(&mut self.kind);
        while let Some(mut e) = next {
            next = take_chain_left(&mut e.kind);
        }
    }
}

/// Takes the left side out of a chain link; the rest of the link drops
/// here.
fn take_chain_left(kind: &mut ExprKind) -> Option<Box<Expr>> {
    chain_left(kind)?;
    match std::mem::replace(kind, ExprKind::Null) {
        ExprKind::Binary(_, l, _) | ExprKind::Logical(_, l, _) => Some(l),
        ExprKind::Member { obj, .. } | ExprKind::Index { obj, .. } => Some(obj),
        ExprKind::Call { callee, .. } => Some(callee),
        _ => None,
    }
}
