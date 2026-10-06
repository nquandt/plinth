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
            | TypeAnn::Object(_, s) => *s,
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
    /// `{ a, b: c }`: `(key, pattern)`.
    Object(Vec<(String, Pattern)>, Span),
    Array(Vec<Option<Pattern>>, Span),
}

impl Pattern {
    pub fn span(&self) -> Span {
        match self {
            Pattern::Ident(_, s) | Pattern::Object(_, s) | Pattern::Array(_, s) => *s,
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
    Empty,
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

#[derive(Debug, Clone)]
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
