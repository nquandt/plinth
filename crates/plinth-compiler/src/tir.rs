//! The typed IR. The checker produces it, `lower` rewrites JSX and the
//! reactive primitives in it into runtime calls, and `codegen` emits Wasm.
//!
//! Every conversion is explicit (`Coerce`), so codegen never infers one.

use crate::diag::Span;
use crate::types::{EnumDef, Repr, StructDef, StructId, Type};
use plinth_protocol::ControlKind;

pub type VarId = u32;
pub type FuncId = u32;
pub type LoopId = u32;

#[derive(Debug, Clone)]
pub struct VarInfo {
    pub name: String,
    pub ty: Type,
    pub owner: FuncId,
    pub mutable: bool,
    /// A module variable lives in the module environment (a rooted global).
    pub module: Option<u32>,
    /// The innermost loop in `owner` that declares the variable. A captured
    /// variable in a loop body gets a new environment for each iteration.
    pub in_loop: Option<LoopId>,
    /// Set by the capture analysis: a function other than `owner` uses it.
    pub captured: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FuncKind {
    /// A top-level function declaration. Its environment is always 0.
    TopLevel,
    /// An arrow function, a function expression, or a nested declaration.
    Closure,
    /// The top-level statements of one module.
    ModuleInit(u32),
}

#[derive(Debug, Clone)]
pub struct FuncDef {
    pub name: String,
    pub kind: FuncKind,
    pub params: Vec<VarId>,
    pub ret: Type,
    pub body: Vec<TStmt>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Screen {
    pub name: String,
    pub icon: Option<String>,
    pub component: FuncId,
    /// A top-level destination, shown in the tab bar / rail / sidebar
    /// (SPEC.md §6.2). A non-primary screen is reachable only through
    /// `navigate.push` (UI API 1.2).
    pub primary: bool,
}

#[derive(Debug, Default)]
pub struct Program {
    pub funcs: Vec<FuncDef>,
    pub vars: Vec<VarInfo>,
    pub structs: Vec<StructDef>,
    pub enums: Vec<EnumDef>,
    /// Module init functions in dependency order.
    pub module_inits: Vec<FuncId>,
    pub module_count: u32,
    pub screens: Vec<Screen>,
    pub accent: Option<String>,
    pub loop_count: u32,
    /// Classes (SPEC.md §4.2), keyed by the struct that holds their fields.
    /// Codegen uses this to resolve `MethodCall` and `InstanceOf` against
    /// the whole program's hierarchy, built after every module is checked.
    pub classes: std::collections::HashMap<StructId, ClassDef>,
    /// The built-in `Error` class (SPEC.md §5.6). Its first two fields are
    /// `name` and `message`, in every subclass too.
    pub error_class: Option<StructId>,
}

/// One class's place in the inheritance hierarchy and its own (not
/// inherited) methods, by name.
#[derive(Debug, Clone, Default)]
pub struct ClassDef {
    pub base: Option<StructId>,
    pub methods: std::collections::HashMap<String, FuncId>,
}

impl Program {
    pub fn new_var(&mut self, info: VarInfo) -> VarId {
        self.vars.push(info);
        (self.vars.len() - 1) as VarId
    }

    pub fn new_func(&mut self, def: FuncDef) -> FuncId {
        self.funcs.push(def);
        (self.funcs.len() - 1) as FuncId
    }

    pub fn new_loop(&mut self) -> LoopId {
        self.loop_count += 1;
        self.loop_count - 1
    }
}

#[derive(Debug, Clone)]
pub enum TStmt {
    /// Declares a variable. `None` initializes it to the zero value.
    Let(VarId, Option<TExpr>),
    Expr(TExpr),
    If(TExpr, Vec<TStmt>, Vec<TStmt>),
    /// `while`, `for` and `do…while` all lower to this loop.
    /// `cond` runs before each iteration unless `test_after`; `update` runs
    /// after each iteration and after `continue`.
    Loop { id: LoopId, cond: Option<TExpr>, test_after: bool, update: Option<TExpr>, body: Vec<TStmt> },
    /// `for (const x of arr)`. `var` is set for each element.
    ForOf { id: LoopId, var: VarId, arr: TExpr, body: Vec<TStmt> },
    Return(Option<TExpr>),
    Break,
    Continue,
    /// The discriminant and the cases; the case values are already checked
    /// for type, `eq` compares them.
    Switch { disc: TExpr, eq: EqKind, cases: Vec<(Option<TExpr>, Vec<TStmt>)> },
    /// `throw e` (SPEC.md §5.6): `e` is an `Error` (or a subclass). A
    /// `catch` can catch it; an uncaught one is reported to the host.
    Throw(TExpr),
    /// A failed run-time check that the compiler generated (a bad `as`
    /// cast): the guest traps with the message. No `catch` can catch it.
    Trap(TExpr),
    /// `try`/`catch`/`finally` (SPEC.md §5.6). The catch variable gets the
    /// thrown `Error`.
    Try { body: Vec<TStmt>, catch: Option<(VarId, Vec<TStmt>)>, finally: Option<Vec<TStmt>> },
    Block(Vec<TStmt>),
}

#[derive(Debug)]
pub struct TExpr {
    pub kind: TExprKind,
    pub ty: Type,
    pub span: Span,
}

impl TExpr {
    pub fn new(kind: TExprKind, ty: Type, span: Span) -> Self {
        Self { kind, ty, span }
    }

    pub fn repr(&self) -> Repr {
        self.ty.repr()
    }

    /// The same expression with the static type `ty`.
    pub fn with_ty(mut self, ty: Type) -> Self {
        self.ty = ty;
        self
    }

    /// The parts of the expression. (`TExpr` has a `Drop`, so a pattern
    /// cannot move them out.)
    pub fn into_parts(mut self) -> (TExprKind, Type, Span) {
        let kind = std::mem::replace(&mut self.kind, TExprKind::Null);
        let ty = std::mem::replace(&mut self.ty, Type::Void);
        (kind, ty, self.span)
    }
}

impl TExprKind {
    /// The left side of a chain link: the part of the expression that runs
    /// first, before all its other parts. A long `a + b + c` or
    /// `x.f().g()` is deep only on this side, so `lower` and codegen walk
    /// such a chain with a loop, not recursion (docs/GAPS.md R3).
    pub fn chain_left(&self) -> Option<&TExpr> {
        match self {
            TExprKind::Num2(_, a, _)
            | TExprKind::Int2(_, a, _)
            | TExprKind::Cmp(_, _, a, _)
            | TExprKind::StrCmp(_, a, _)
            | TExprKind::Concat(a, _)
            | TExprKind::And(a, _)
            | TExprKind::Or(a, _)
            | TExprKind::Field(a, ..)
            | TExprKind::Index(a, _)
            | TExprKind::MethodCall(_, _, a, _) => Some(a),
            TExprKind::Call(_, args) => args.first(),
            _ => None,
        }
    }

    /// Like `chain_left`, for changing it.
    pub fn chain_left_mut(&mut self) -> Option<&mut TExpr> {
        match self {
            TExprKind::Num2(_, a, _)
            | TExprKind::Int2(_, a, _)
            | TExprKind::Cmp(_, _, a, _)
            | TExprKind::StrCmp(_, a, _)
            | TExprKind::Concat(a, _)
            | TExprKind::And(a, _)
            | TExprKind::Or(a, _)
            | TExprKind::Field(a, ..)
            | TExprKind::Index(a, _)
            | TExprKind::MethodCall(_, _, a, _) => Some(a),
            TExprKind::Call(_, args) => args.first_mut(),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
}

/// Arithmetic on two `int` values (SPEC.md §4.2): stays `i32`, unlike
/// `NumOp` which always works on `f64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// How two values compare for equality.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EqKind {
    F64,
    I32,
    Str,
    /// Reference identity (also used for `=== null`).
    Ref,
    /// `string | null` (first) with `string`: false when the first is null.
    NullStr,
    /// `number | null` (first, boxed) with `number`.
    NullF64,
    /// An `int | null` / `boolean | null` / enum `| null` (first, boxed,
    /// through the same box as `number | null`) with the plain scalar.
    NullI32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coercion {
    /// number → number | null
    BoxNum,
    /// number | null → number (traps on null)
    UnboxNum,
    /// A scalar of `i32` representation (`int`, `boolean`, an enum) →
    /// the same type `| null` (HANDOFF.md item 2). Reuses the `number`
    /// box: converts to `f64`, then `box_f64` (exact for every `i32`).
    BoxI32,
    /// The reverse of `BoxI32` (traps on null): `unbox_f64`, then
    /// truncates back to `i32`.
    UnboxI32,
    NumToStr,
    BoolToStr,
    /// An enum or another i32 to a number.
    I32ToNum,
    /// number → i32 (truncates)
    NumToI32,
    /// Any value to a boolean (JS truthiness).
    Truthy,
    /// A value of the same representation with a new static type.
    Retag,
    /// Drops the value.
    Discard,
}

#[derive(Debug, Clone)]
pub enum Place {
    Var(VarId),
    Field(Box<TExpr>, StructId, u32),
    Index(Box<TExpr>, Box<TExpr>),
}

/// Array methods that take a callback. Codegen emits an inline loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrayHof {
    Map,
    Filter,
    Find,
    FindIndex,
    Some,
    Every,
    ForEach,
}

#[derive(Debug, Clone)]
pub enum TExprKind {
    Num(f64),
    Bool(bool),
    Str(String),
    /// `null` of a reference type, or "no element".
    Null,
    Var(VarId),
    Assign(Place, Box<TExpr>),
    Field(Box<TExpr>, StructId, u32),
    Index(Box<TExpr>, Box<TExpr>),
    /// A direct call of a top-level function.
    Call(FuncId, Vec<TExpr>),
    /// A call of a closure value.
    CallClosure(Box<TExpr>, Vec<TExpr>),
    /// A closure value for a function.
    Closure(FuncId),
    Num2(NumOp, Box<TExpr>, Box<TExpr>),
    Int2(IntOp, Box<TExpr>, Box<TExpr>),
    Neg(Box<TExpr>),
    Not(Box<TExpr>),
    Cmp(CmpOp, EqKind, Box<TExpr>, Box<TExpr>),
    /// String comparison for `<`, `<=`, `>`, `>=`.
    StrCmp(CmpOp, Box<TExpr>, Box<TExpr>),
    Concat(Box<TExpr>, Box<TExpr>),
    And(Box<TExpr>, Box<TExpr>),
    Or(Box<TExpr>, Box<TExpr>),
    Cond(Box<TExpr>, Box<TExpr>, Box<TExpr>),
    IsNull(Box<TExpr>),
    /// Reads a discriminated union's shared literal field (the first field
    /// of every member struct, so every member has it at the same offset).
    UnionTag(Box<TExpr>),
    /// True if the union value's runtime member is one of these indices
    /// into the `Type::Union` member list of the operand's static type.
    UnionIs(Box<TExpr>, Vec<usize>),
    /// A class method call through a value whose static type is `StructId`
    /// (SPEC.md §4.2 v1). Codegen resolves this to a direct call when no
    /// reachable subclass overrides the method, or to an inline dispatch on
    /// the receiver's runtime type id otherwise.
    MethodCall(StructId, String, Box<TExpr>, Vec<TExpr>),
    /// `x instanceof C`: true if the receiver's runtime type is `C` or one
    /// of its subclasses.
    InstanceOf(Box<TExpr>, StructId),
    Coerce(Coercion, Box<TExpr>),
    /// Statements, then a value.
    Block(Vec<TStmt>, Box<TExpr>),
    ArrayLit(Vec<(bool, TExpr)>),
    StructLit(StructId, Vec<TExpr>),
    ArrayHof { kind: ArrayHof, arr: Box<TExpr>, f: Box<TExpr>, arity: usize },
    /// `includes` and `indexOf`.
    ArraySearch { index: bool, eq: EqKind, arr: Box<TExpr>, value: Box<TExpr> },
    /// A runtime function call (`rt_abi::FUNCTIONS`).
    Rt(&'static str, Vec<TExpr>),
    /// A Wasm instruction on f64 values.
    MathOp(MathOp, Vec<TExpr>),
    /// `await p` (SPEC.md §4.5) in an `async` function. The checker
    /// rewrites the function into continuations (`check/asyncfn.rs`), so no
    /// later phase sees it.
    Await(Box<TExpr>),

    // -- Reactive and UI forms. `lower` replaces all of these. -------------
    SignalNew(Box<TExpr>),
    SignalGet(Box<TExpr>),
    SignalPeek(Box<TExpr>),
    SignalSet(Box<TExpr>, Box<TExpr>),
    ComputedNew(Box<TExpr>),
    ComputedGet(Box<TExpr>),
    EffectNew(Box<TExpr>),
    Jsx(Box<TJsx>),
    /// `navigate("name")`. The screen index is resolved after the app config.
    Navigate(String),
    /// `navigate.push("name")` (UI API 1.2). The screen index is resolved
    /// after the app config, like `Navigate`.
    NavigatePush(String),
    /// `navigate.back()` (UI API 1.2).
    NavigateBack,
    /// `setTimeout`/`setInterval`: `(ms, repeat, callback)`. Returns the
    /// timer id as a `number` (SPEC.md §8.5, `plinth:time`).
    TimerNew(Box<TExpr>, bool, Box<TExpr>),
    /// A runtime function that takes a callback and nothing else (core
    /// 1.12): `onFrame` (`set_frame`), `onCleanup` (`on_cleanup`). Lowers to
    /// `Rt(name, [thunk, callback])`.
    RtCallback(&'static str, Box<TExpr>),
    /// `plinth:dialog`'s `alert`/`confirm`/`prompt`: `(rt function name,
    /// message, done callback)`. Returns `void`; the done callback carries
    /// the result (SPEC.md §8.4, §8.5).
    DialogCall(&'static str, Box<TExpr>, Box<TExpr>),
    /// `plinth:net`'s `fetch`: `(url, method, headers, body, wrapper)`.
    /// `headers` is a `Map<string, string>` value or `Null`; `body` is a
    /// nullable string. `wrapper` is a synthetic 0-arg closure the checker
    /// built (`check/stdlib.rs`) that decodes the completion result into a
    /// `Response` and calls the app's `done` (SPEC.md §8.4, §8.5, §11).
    NetFetchCall(Box<TExpr>, Box<TExpr>, Box<TExpr>, Box<TExpr>, Box<TExpr>),
    /// A host call with an operation code and two strings that finishes
    /// through a completion: `(runtime function, op, a, b, wrapper)`.
    /// `plinth:files` (core 1.11) is `files_call` with `(op, path, text)`;
    /// `text` is a nullable string (only `write` uses it). `store.sql`
    /// (core 1.13) is `sql_call` with `(kind, statement, params)`.
    /// `wrapper` is a synthetic 0-arg closure that reads the completion with
    /// `net_result_*` (`check/stdlib.rs`, `check/sql.rs`).
    HostCall(&'static str, i32, Box<TExpr>, Box<TExpr>, Box<TExpr>),
    /// `plinth:notify` `show` (core 1.12): `(title, body, tag, wrapper)`.
    /// `body` and `tag` are nullable strings. `wrapper` is a synthetic 0-arg
    /// closure that reads the completion with `net_result_*`
    /// (`check/stdlib.rs`).
    NotifyShow(Box<TExpr>, Box<TExpr>, Box<TExpr>, Box<TExpr>),

    // -- Forms that only `lower` makes. ------------------------------------
    /// The table index of the thunk adapter for a closure signature.
    ThunkOf(ThunkSig),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathOp {
    Floor,
    Ceil,
    Trunc,
    Abs,
    Sqrt,
    Min,
    Max,
}

/// The signature of a closure that the runtime calls through a thunk.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ThunkSig {
    pub params: Vec<Repr>,
    pub ret: Repr,
}

// -- JSX (checked, not lowered) -------------------------------------------

#[derive(Debug, Clone)]
pub enum TJsx {
    Control { kind: ControlKind, props: Vec<TProp>, children: TChildren, span: Span },
    /// A user component: a direct call with one optional props object.
    Component { func: FuncId, props: Option<TExpr>, span: Span },
}

#[derive(Debug, Clone)]
pub struct TProp {
    pub target: PropTarget,
    pub value: TExpr,
}

#[derive(Debug, Clone)]
pub enum PropTarget {
    Str(u16),
    Num(u16),
    Int(u16),
    Bool(u16),
    /// A string literal union mapped to enum ids.
    Enum(u16, Vec<(String, u16)>),
    Event(u16),
    /// A two-way binding to a signal (`value={sig}`).
    Bind { kind: BindValKind },
    ListItems,
    ListKey,
    ListRow,
    ListEmpty,
}

/// The value type of a two-way binding (SPEC.md §8.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindValKind {
    Str,
    Bool,
    Num,
}

#[derive(Debug, Clone)]
pub enum TChildren {
    None,
    /// Text content: string parts, concatenated.
    Text(Vec<TExpr>),
    Nodes(Vec<TChild>),
}

#[derive(Debug, Clone)]
pub enum TChild {
    Element(TJsx),
    /// An expression of type `Element` (0 means none).
    Expr(TExpr),
}

/// Copies a left-deep chain (`TExprKind::chain_left`) with a loop, not
/// recursion (docs/GAPS.md R3).
impl Clone for TExpr {
    fn clone(&self) -> Self {
        let mut spine = Vec::new();
        let mut cur = self;
        while let Some(left) = cur.kind.chain_left() {
            spine.push(cur);
            cur = left;
        }
        let mut acc = TExpr { kind: cur.kind.clone(), ty: cur.ty.clone(), span: cur.span };
        while let Some(node) = spine.pop() {
            acc = TExpr { kind: node.kind.clone_link(acc), ty: node.ty.clone(), span: node.span };
        }
        acc
    }
}

impl TExprKind {
    /// A copy of a chain link with `left` as its left side.
    fn clone_link(&self, left: TExpr) -> TExprKind {
        let l = Box::new(left);
        let bx = |e: &TExpr| Box::new(e.clone());
        match self {
            TExprKind::Num2(op, _, b) => TExprKind::Num2(*op, l, bx(b)),
            TExprKind::Int2(op, _, b) => TExprKind::Int2(*op, l, bx(b)),
            TExprKind::Cmp(op, k, _, b) => TExprKind::Cmp(*op, *k, l, bx(b)),
            TExprKind::StrCmp(op, _, b) => TExprKind::StrCmp(*op, l, bx(b)),
            TExprKind::Concat(_, b) => TExprKind::Concat(l, bx(b)),
            TExprKind::And(_, b) => TExprKind::And(l, bx(b)),
            TExprKind::Or(_, b) => TExprKind::Or(l, bx(b)),
            TExprKind::Field(_, s, i) => TExprKind::Field(l, *s, *i),
            TExprKind::Index(_, i) => TExprKind::Index(l, bx(i)),
            TExprKind::MethodCall(sid, name, _, args) => TExprKind::MethodCall(*sid, name.clone(), l, args.clone()),
            TExprKind::Call(f, args) => {
                let mut out = vec![*l];
                out.extend(args[1..].iter().cloned());
                TExprKind::Call(*f, out)
            }
            other => unreachable!("not a chain link: {other:?}"),
        }
    }
}

/// Drops a left-deep chain (`TExprKind::chain_left`) with a loop. The
/// default drop recurses once per link, and a chain has no length limit
/// (docs/GAPS.md R3).
impl Drop for TExpr {
    fn drop(&mut self) {
        let mut next = take_chain_left(&mut self.kind);
        while let Some(mut e) = next {
            next = take_chain_left(&mut e.kind);
        }
    }
}

/// Takes the left side out of a chain link; the rest of the link drops
/// here.
fn take_chain_left(kind: &mut TExprKind) -> Option<TExpr> {
    let left = kind.chain_left_mut()?;
    Some(std::mem::replace(left, TExpr::new(TExprKind::Null, Type::Void, Span::default())))
}
