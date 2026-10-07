//! The Plinth TS type checker (SPEC.md §5.1 step 3).
//!
//! It checks the modules in dependency order and produces the typed IR
//! (`tir::Program`). Each rejected feature has a stable code (`diag::code`).

mod asyncfn;
mod expr;
mod promises;
mod jsx;
pub mod stdlib;
mod stmt;

use crate::ast::{self, Item, TypeAnn};
use crate::diag::{Diagnostic, FileId, Span, code};
use crate::tir::*;
use crate::types::{self, EnumDef, Field, FuncType, StructDef, Type};
use plinth_protocol::ControlKind;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// A `plinth:*` module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StdModule {
    Ui,
    Core,
    Time,
    Store,
    Clipboard,
    Dialog,
    Net,
    Hub,
    Files,
}

impl StdModule {
    pub fn from_specifier(s: &str) -> Option<Self> {
        match s {
            "plinth:ui" => Some(StdModule::Ui),
            "plinth:core" => Some(StdModule::Core),
            "plinth:time" => Some(StdModule::Time),
            "plinth:store" => Some(StdModule::Store),
            "plinth:clipboard" => Some(StdModule::Clipboard),
            "plinth:dialog" => Some(StdModule::Dialog),
            "plinth:net" => Some(StdModule::Net),
            "plinth:hub" => Some(StdModule::Hub),
            "plinth:files" => Some(StdModule::Files),
            _ => None,
        }
    }
}

/// Where an import points.
#[derive(Debug, Clone, Copy)]
pub enum ModRef {
    User(usize),
    Std(StdModule),
}

/// One parsed module, ready to check.
pub struct ModuleSrc {
    pub file: FileId,
    pub ast: ast::Module,
    /// One entry for each `Item::Import`, in order. `None` if unresolved.
    pub imports: Vec<Option<ModRef>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StdFn {
    Signal,
    Computed,
    Effect,
    /// `onCleanup(fn)` (core 1.12): runs `fn` when the current component or region goes away.
    OnCleanup,
    /// `isActive()` (core 1.12): a reactive boolean, true while the app's window is in the foreground.
    IsActive,
    App,
    Navigate,
    ParseNumber,
    /// `seedRandom(seed)` (core 1.12): the same seed gives the same `Math.random` numbers.
    SeedRandom,
    ToString,
    TimeNow,
    TimeMonotonicNow,
    SetTimeout,
    SetInterval,
    OnFrame,
    ClearTimer,
    ClipboardWriteText,
    ClipboardReadText,
    Int,
    ClipboardLastError,
    DialogAlert,
    DialogConfirm,
    DialogPrompt,
    NetFetch,
    TimezoneOffset,
    DateParts,
    MakeDate,
    FormatDate,
    ToIsoString,
    ParseDate,
    HubListApps,
    HubLaunch,
    HubSetGrant,
    HubBlock,
    HubUnblock,
    HubLastError,
    HubListGroups,
    HubCreateGroup,
    HubSetGroup,
    HubRemove,
    HubSearch,
    HubInstall,
    HubAppInfo,
    HubPin,
    HubBlockPublisher,
    HubUnblockPublisher,
    HubCheckUpdates,
    HubUpdate,
    FilesRead,
    FilesWrite,
    FilesList,
    FilesStat,
    FilesRemove,
    /// `rect`, `circle`, `line`, `canvasText` from `plinth:ui` (UI API 1.10):
    /// each makes one encoded `Shape` string for `Canvas.shapes`.
    ShapeRect,
    ShapeCircle,
    ShapeLine,
    ShapeText,
    /// `strokeRect`, `strokeCircle` (UI API 1.13): outlines.
    ShapeStrokeRect,
    ShapeStrokeCircle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StdObj {
    Math,
    Console,
    /// `navigate`: callable directly (`navigate("name")`, `Binding::Std`
    /// resolves that call) and also has `.push`/`.back` members (UI API 1.2).
    Navigate,
    /// `kv` from `plinth:store` (SPEC.md §8.5, capability `store.kv`).
    Kv,
    /// `JSON` from `plinth:core` (SPEC.md §4.7): `stringify`/`parse`.
    Json,
}

#[derive(Debug, Clone)]
pub enum Binding {
    Var(VarId),
    Func(FuncId),
    Type(Type),
    /// A type alias that is not resolved yet: `(module, alias index)`.
    Alias(usize, usize),
    /// A generic interface template, not monomorphized yet:
    /// `(module, interface index)`.
    Interface(usize, usize),
    Enum(types::EnumId),
    Control(ControlKind),
    Std(StdFn),
    StdObj(StdObj),
    /// A generic function template, by index into `Checker::generics`.
    Generic(usize),
}

/// A narrowing key: a plain variable (`x`, empty path) or a member path
/// rooted at a `const`/parameter variable (`x.a` is `(x, [idx_a])`, `x.a.b`
/// is `(x, [idx_a, idx_b])`). See `docs/language.md` ("Narrowing").
pub(crate) type NarrowKey = (VarId, Vec<u32>);

#[derive(Default)]
struct Scope {
    names: HashMap<String, Binding>,
    /// Narrowed types of variables and member paths (`if (x !== null)`,
    /// `if (r.subtitle !== null)`).
    narrow: HashMap<NarrowKey, Type>,
}

/// Where a signal/computed read happens, for the "not reactive" lint
/// (HANDOFF.md §6: a read outside JSX, `computed` or `effect` does not
/// re-run when the signal changes).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ReactiveCtx {
    /// Plain statements in a function body: a signal read here runs once
    /// and never again, which is usually a bug.
    Plain,
    /// Inside JSX (a prop or child expression), or inside `computed`/`effect`.
    Reactive,
    /// Inside an event handler or other callback: reads are intentional
    /// (they see the current value when the callback runs), so no warning.
    Callback,
}

/// The state of the function that the checker is in.
struct FnCx {
    func: FuncId,
    scopes: Vec<Scope>,
    loops: Vec<LoopId>,
    /// The declared return type, or `None` to infer it.
    ret: Option<Type>,
    /// The inferred return type so far.
    inferred: Option<Type>,
    reactive: ReactiveCtx,
    /// The function is `async`: `await` is allowed, and `ret` is the `T`
    /// of its `Promise<T>` (SPEC.md §4.5).
    is_async: bool,
}

struct PendingFunc {
    decl: ast::FuncDecl,
    module: usize,
}

pub struct Checker<'d> {
    pub prog: Program,
    diags: &'d mut Vec<Diagnostic>,
    exports: Vec<HashMap<String, Binding>>,
    defaults: Vec<Option<Binding>>,
    /// The module scope of each module (top-level names).
    module_scopes: Vec<HashMap<String, Binding>>,
    module: usize,
    fx: FnCx,
    pending: HashMap<FuncId, PendingFunc>,
    in_progress: HashSet<FuncId>,
    aliases: Vec<Vec<ast::TypeAlias>>,
    resolving_alias: HashSet<(usize, usize)>,
    /// Generic interface templates, not monomorphized until a type
    /// annotation instantiates them with type arguments.
    interfaces: Vec<Vec<ast::Interface>>,
    resolving_interface: HashSet<(usize, usize)>,
    anon_structs: HashMap<String, types::StructId>,
    /// The screen names that `navigate` and `navigate.push` refer to,
    /// checked after the app. The `bool` is `true` for `navigate(name)`,
    /// which needs a primary screen; `navigate.push` accepts any screen.
    pub navigations: Vec<(String, Span, bool)>,
    app_seen: bool,
    /// The capability names declared in `plinth.toml` (SPEC.md §11). A
    /// host API call that needs a capability not in this set is a compile
    /// error (`code::CAPABILITY_UNDECLARED`).
    capabilities: HashSet<String>,
    /// Set just before checking a `computed`/`effect` callback body, so the
    /// new closure's `FnCx` starts in `Reactive` instead of `Callback`.
    pending_reactive: bool,
    /// Generic top-level function templates, not checked until a call site
    /// instantiates them (HANDOFF.md item 3).
    generics: Vec<GenericTemplate>,
    /// Concrete types for the generic function template being checked
    /// right now, by type-parameter name.
    generic_bindings: HashMap<String, Type>,
    /// Instantiations so far: `(template index, type arguments) -> FuncId`.
    /// Linear (there are only ever a handful) so `Type` need not be `Hash`.
    instantiations: Vec<(usize, Vec<Type>, FuncId)>,
    /// Instantiations of generic type aliases so far:
    /// `(module, alias index, type arguments) -> resolved type`. Linear for
    /// the same reason as `instantiations`.
    alias_instantiations: Vec<(usize, usize, Vec<Type>, Type)>,
    /// Instantiations of generic interfaces so far:
    /// `(module, interface index, type arguments) -> monomorphized struct`.
    interface_instantiations: Vec<(usize, usize, Vec<Type>, types::StructId)>,
    /// Basic classes (SPEC.md §4.2): the nominal struct's fields are the
    /// class fields; each method lowers to a top-level function with a
    /// synthetic `this` first parameter (static dispatch, v0: no `extends`).
    classes: HashMap<types::StructId, ClassInfo>,
    /// Asset paths under `assets/` in the project (without the `assets/`
    /// prefix), for checking `<Image src>` (SPEC.md §6.3, §10.1).
    assets: HashSet<String>,
    /// The built-in `Error` class (SPEC.md §5.6), a global name in every
    /// module unless the module declares its own `Error`.
    pub(crate) error_class: types::StructId,
    /// `Promise<T>` structs and their helper functions, one per `T`
    /// (`check/asyncfn.rs`).
    promises: Vec<asyncfn::PromiseInfo>,
    /// The app's microtask queue and drain, made with the first promise.
    async_rt: Option<asyncfn::AsyncRt>,
    /// `Promise.all` helper functions, one per element type
    /// (`check/promises.rs`).
    promise_alls: Vec<(promises::Combinator, Type, FuncId)>,
}

/// The built-in `Error` class (SPEC.md §5.6). `throw` takes an instance of
/// it (or of a subclass), and a `catch` variable has its type. Its fields
/// are `name` and `message`, in this order.
const ERROR_PRELUDE: &str = "class Error { name: string = \"Error\"; message: string; constructor(message?: string) { this.message = message ?? \"\"; } }";

/// A tuple: the types of its fixed elements (an optional one is
/// `T | null`), how many of them are required, and the element type of its
/// rest element.
pub(crate) struct TupleShape {
    pub fixed: Vec<Type>,
    pub required: usize,
    pub rest: Option<Type>,
}

impl TupleShape {
    /// The index of the field that holds the rest elements.
    pub fn rest_field(&self) -> u32 {
        self.fixed.len() as u32
    }
}

struct ClassInfo {
    /// `None` only right after a "a class needs a constructor" error.
    ctor: Option<FuncId>,
    /// This class's own methods (not inherited ones), by name.
    methods: HashMap<String, FuncId>,
    /// The base class, if `extends` names one that resolved (SPEC.md §4.2
    /// v1: single inheritance).
    base: Option<types::StructId>,
}

struct GenericTemplate {
    decl: ast::FuncDecl,
    module: usize,
}

/// Checks all modules. `modules` must be in dependency order (dependencies
/// first); `main` is the index of `app/main.tsx`. `capabilities` are the
/// capability names declared in the project's `plinth.toml`.
pub fn check(modules: &[ModuleSrc], main: usize, diags: &mut Vec<Diagnostic>, capabilities: &[String]) -> Program {
    check_ex(modules, main, diags, capabilities, &[])
}

/// Like [`check`], with the asset paths under `assets/` (SPEC.md §10.1),
/// for checking `<Image src>`.
pub fn check_ex(modules: &[ModuleSrc], main: usize, diags: &mut Vec<Diagnostic>, capabilities: &[String], assets: &[String]) -> Program {
    let mut c = Checker {
        prog: Program::default(),
        diags,
        exports: vec![HashMap::new(); modules.len()],
        defaults: vec![None; modules.len()],
        module_scopes: vec![HashMap::new(); modules.len()],
        module: 0,
        fx: FnCx { func: 0, scopes: Vec::new(), loops: Vec::new(), ret: None, inferred: None, reactive: ReactiveCtx::Plain, is_async: false },
        pending: HashMap::new(),
        in_progress: HashSet::new(),
        aliases: vec![Vec::new(); modules.len()],
        resolving_alias: HashSet::new(),
        interfaces: vec![Vec::new(); modules.len()],
        resolving_interface: HashSet::new(),
        anon_structs: HashMap::new(),
        navigations: Vec::new(),
        app_seen: false,
        capabilities: capabilities.iter().cloned().collect(),
        pending_reactive: false,
        generics: Vec::new(),
        generic_bindings: HashMap::new(),
        instantiations: Vec::new(),
        alias_instantiations: Vec::new(),
        interface_instantiations: Vec::new(),
        classes: HashMap::new(),
        assets: assets.iter().cloned().collect(),
        error_class: 0,
        promises: Vec::new(),
        async_rt: None,
        promise_alls: Vec::new(),
    };
    c.prog.module_count = modules.len() as u32;
    c.declare_error_class(modules[main].file);
    for (i, m) in modules.iter().enumerate() {
        c.check_module(i, m, i == main);
    }
    if !c.app_seen {
        let file = modules[main].file;
        c.diags.push(
            Diagnostic::error(code::NO_APP, Span::new(file, 0, 0), "app/main.tsx must `export default app({...})`")
                .help("see `plinth new` for a starting point"),
        );
    }
    let primary: Vec<String> = c.prog.screens.iter().filter(|s| s.primary).map(|s| s.name.clone()).collect();
    let all: Vec<String> = c.prog.screens.iter().map(|s| s.name.clone()).collect();
    for (name, span, needs_primary) in std::mem::take(&mut c.navigations) {
        let (ok, names, what) =
            if needs_primary { (primary.contains(&name), &primary, "primary screen") } else { (all.contains(&name), &all, "screen") };
        if !ok {
            c.diags.push(
                Diagnostic::error(code::BAD_NAVIGATE, span, format!("there is no {what} named \"{name}\""))
                    .help(format!("the {what}s are: {}", names.join(", "))),
            );
        }
    }
    for (sid, info) in &c.classes {
        c.prog.classes.insert(*sid, ClassDef { base: info.base, methods: info.methods.clone() });
    }
    c.prog
}

impl Checker<'_> {
    // -- Diagnostics ------------------------------------------------------

    fn err(&mut self, code: &'static str, span: Span, msg: impl Into<String>) {
        self.diags.push(Diagnostic::error(code, span, msg));
    }

    fn err_help(&mut self, code: &'static str, span: Span, msg: impl Into<String>, help: impl Into<String>) {
        self.diags.push(Diagnostic::error(code, span, msg).help(help));
    }

    /// Reports `code::CAPABILITY_UNDECLARED` if `plinth.toml` does not
    /// declare `capability` (SPEC.md §11). Called at each host API call
    /// site that needs it.
    pub(super) fn require_capability(&mut self, capability: &str, span: Span) {
        if !self.capabilities.contains(capability) {
            self.err_help(
                code::CAPABILITY_UNDECLARED,
                span,
                format!("this call needs the `{capability}` capability, which `plinth.toml` does not declare"),
                format!("add `[[capabilities]]` with `name = \"{capability}\"` and a `rationale` to plinth.toml (SPEC.md §11)"),
            );
        }
    }

    /// Reports `code::CAPABILITY_UNDECLARED` unless the manifest declares
    /// at least one `net:<host>` or `net.local` capability (SPEC.md §11).
    /// Capability names for `plinth:net` are dynamic (one per declared
    /// host), so this reachability check can only confirm that `net` is
    /// reachable at all; the real per-host decision happens at call time
    /// in the runner's `Policy` (`crates/plinth-runner-wasmtime`).
    pub(super) fn require_net_capability(&mut self, span: Span) {
        let ok = self.capabilities.iter().any(|c| c == plinth_link::capabilities::NET_LOCAL || c.starts_with("net:"));
        if !ok {
            self.err_help(
                code::CAPABILITY_UNDECLARED,
                span,
                "this call needs a `net:<host>` (or `net.local`) capability, which `plinth.toml` does not declare",
                "add `[[capabilities]]` with `name = \"net:api.example.com\"` and a `rationale` to plinth.toml (SPEC.md §11)",
            );
        }
    }

    /// The memoized `{ ok: boolean; status: number; text: string; error:
    /// string | null }` struct that `net.fetch`'s `done` callback receives
    /// (SPEC.md §8.5). Built ad hoc (not from `std/net.d.ts`'s `Response`
    /// interface, which exists for the editor/`tsc` only): the callback's
    /// parameter type is inferred from context, like `dialog.confirm`'s
    /// `ok: boolean`.
    pub(crate) fn response_struct(&mut self) -> types::StructId {
        let fields = vec![
            Field { name: "ok".into(), ty: Type::Bool, optional: false },
            Field { name: "status".into(), ty: Type::Number, optional: false },
            Field { name: "text".into(), ty: Type::String, optional: false },
            Field { name: "error".into(), ty: Type::String.nullable(), optional: false },
        ];
        match self.anon_struct(fields) {
            Type::Struct(s) => s,
            _ => unreachable!(),
        }
    }

    /// The memoized `{ name: string; kind: string; size: number }` struct
    /// of a `plinth:files` entry (`list`, `stat`; `std/files.d.ts`'s
    /// `FileEntry`), decoded from the host's JSON.
    pub(crate) fn file_entry_struct(&mut self) -> types::StructId {
        let fields = vec![
            Field { name: "name".into(), ty: Type::String, optional: false },
            Field { name: "kind".into(), ty: Type::String, optional: false },
            Field { name: "size".into(), ty: Type::Number, optional: false },
        ];
        match self.anon_struct(fields) {
            Type::Struct(s) => s,
            _ => unreachable!(),
        }
    }

    /// The memoized `DateParts` struct that `dateParts` returns (SPEC.md
    /// §4.7, `plinth:time`, docs/GAPS.md gap #5): `{ year, month, day,
    /// hour, minute, second, millisecond, weekday }`, all `number` (field
    /// order matches `date_field`'s ABI `field` index, `check/stdlib.rs`).
    pub(crate) fn date_parts_struct(&mut self) -> types::StructId {
        let fields = ["year", "month", "day", "hour", "minute", "second", "millisecond", "weekday"]
            .iter()
            .map(|n| Field { name: (*n).into(), ty: Type::Number, optional: false })
            .collect();
        match self.anon_struct(fields) {
            Type::Struct(s) => s,
            _ => unreachable!(),
        }
    }

    /// A synthetic `() => { ...body }` closure, built directly from typed
    /// IR rather than parsed source (mirrors `lower.rs`'s `synthetic`, for
    /// the checker phase: `net.fetch`'s completion wrapper needs one before
    /// `lower` ever runs, since it embeds the already-checked `done`
    /// callback value).
    pub(crate) fn synthetic_closure(&mut self, name: &str, body: Vec<TStmt>, ret: Type, span: Span) -> TExpr {
        let fid = self.prog.new_func(FuncDef { name: name.to_owned(), kind: FuncKind::Closure, params: Vec::new(), ret, body, span });
        let ft = FuncType { params: Vec::new(), required: 0, ret: self.prog.funcs[fid as usize].ret.clone() };
        TExpr::new(TExprKind::Closure(fid), Type::Func(Rc::new(ft)), span)
    }

    fn show(&self, ty: &Type) -> String {
        types::Display { ty, structs: &self.prog.structs, enums: &self.prog.enums }.to_string()
    }

    // -- Modules ----------------------------------------------------------

    fn check_module(&mut self, m: usize, src: &ModuleSrc, is_main: bool) {
        self.module = m;
        let init = self.prog.new_func(FuncDef {
            name: format!("<module {m}>"),
            kind: FuncKind::ModuleInit(m as u32),
            params: Vec::new(),
            ret: Type::Void,
            body: Vec::new(),
            span: Span::new(src.file, 0, 0),
        });
        self.prog.module_inits.push(init);
        self.fx =
            FnCx { func: init, scopes: Vec::new(), loops: Vec::new(), ret: Some(Type::Void), inferred: None, reactive: ReactiveCtx::Plain, is_async: false };

        // 1. Imports.
        let mut import_index = 0;
        for item in &src.ast.items {
            if let Item::Import(imp) = item {
                let target = src.imports.get(import_index).copied().flatten();
                import_index += 1;
                if let Some(target) = target {
                    self.bind_import(imp, target);
                }
            }
        }

        // 2. Types: enums, interfaces and classes first (by name), then
        // fields/methods.
        let mut interfaces = Vec::new();
        let mut classes = Vec::new();
        for item in &src.ast.items {
            match item {
                Item::Enum(e) => self.declare_enum(e),
                Item::Class(c) => {
                    let id = self.prog.structs.len() as types::StructId;
                    self.prog.structs.push(StructDef { name: c.name.clone(), fields: Vec::new() });
                    self.define(&c.name, c.span, Binding::Type(Type::Struct(id)));
                    if c.exported {
                        self.exports[m].insert(c.name.clone(), Binding::Type(Type::Struct(id)));
                    }
                    classes.push((id, c));
                }
                Item::Interface(i) if i.type_params.is_empty() => {
                    let id = self.prog.structs.len() as types::StructId;
                    self.prog.structs.push(StructDef { name: i.name.clone(), fields: Vec::new() });
                    self.define(&i.name, i.span, Binding::Type(Type::Struct(id)));
                    if i.exported {
                        self.exports[m].insert(i.name.clone(), Binding::Type(Type::Struct(id)));
                    }
                    interfaces.push((id, i));
                }
                Item::Interface(i) => {
                    let idx = self.interfaces[m].len();
                    self.interfaces[m].push(i.clone());
                    self.define(&i.name, i.span, Binding::Interface(m, idx));
                    if i.exported {
                        self.exports[m].insert(i.name.clone(), Binding::Interface(m, idx));
                    }
                }
                Item::TypeAlias(a) => {
                    let idx = self.aliases[m].len();
                    self.aliases[m].push(a.clone());
                    self.define(&a.name, a.span, Binding::Alias(m, idx));
                    if a.exported {
                        self.exports[m].insert(a.name.clone(), Binding::Alias(m, idx));
                    }
                }
                _ => {}
            }
        }
        for (id, i) in interfaces {
            let fields = self.fields(&i.fields);
            self.prog.structs[id as usize].fields = fields;
        }
        for (id, c) in classes {
            self.declare_class(id, c, m);
        }

        // 3. Top-level functions are hoisted.
        for item in &src.ast.items {
            if let Item::Stmt(ast::Stmt { kind: ast::StmtKind::Func(f), .. }) = item {
                if f.type_params.is_empty() {
                    self.declare_top_func(f, m);
                } else {
                    self.declare_generic(f, m);
                }
            }
        }

        // 4. Top-level statements, in order, form the module init.
        let mut body = Vec::new();
        for item in &src.ast.items {
            match item {
                Item::Stmt(ast::Stmt { kind: ast::StmtKind::Func(_), .. }) => {}
                Item::Stmt(s) => {
                    let stmts = self.stmt(s, true);
                    body.extend(stmts);
                }
                Item::ExportDefault(e) => {
                    if is_main {
                        self.app_config(e);
                    } else {
                        let te = self.expr(e, None);
                        if let TExprKind::Closure(f) = te.kind {
                            self.defaults[m] = Some(Binding::Func(f));
                        } else {
                            self.err_help(
                                code::UNSUPPORTED,
                                e.span,
                                "only functions can be default exports",
                                "export a named `const` instead",
                            );
                        }
                    }
                }
                Item::ExportNames(names) => {
                    for (local, exported, span) in names {
                        match self.lookup(local) {
                            Some(b) => {
                                self.exports[m].insert(exported.clone(), b);
                            }
                            None => self.err(code::UNKNOWN_NAME, *span, format!("`{local}` is not defined")),
                        }
                    }
                }
                _ => {}
            }
        }
        // 5. The bodies of the functions that no one needed early.
        let mut pending: Vec<FuncId> =
            self.pending.iter().filter(|(_, p)| p.module == m).map(|(f, _)| *f).collect();
        pending.sort();
        for f in pending {
            self.check_pending(f);
        }
        self.prog.funcs[init as usize].body = body;
        if is_main && self.defaults[m].is_none() && !self.app_seen {
            // Reported once in `check`.
        }
    }

    fn bind_import(&mut self, imp: &ast::Import, target: ModRef) {
        match target {
            ModRef::Std(sm) => {
                if let Some((_, span)) = &imp.default {
                    self.err(code::UNKNOWN_EXPORT, *span, "Plinth modules have no default export");
                }
                for (imported, local, span) in &imp.names {
                    // `ChartPoint`/`ChartSeriesDef` are real object types, so
                    // that an app can build `Chart` data in a variable.
                    let chart_type = match (sm, imported.as_str()) {
                        (StdModule::Ui, "ChartPoint") => Some(self.chart_point_type()),
                        (StdModule::Ui, "ChartSeriesDef") => {
                            let points = Type::Array(Box::new(self.chart_point_type()));
                            Some(self.anon_struct(vec![
                                Field { name: "name".into(), ty: Type::String, optional: false },
                                Field { name: "points".into(), ty: points, optional: false },
                            ]))
                        }
                        _ => None,
                    };
                    if let Some(t) = chart_type {
                        self.define(local, *span, Binding::Type(t));
                        continue;
                    }
                    match stdlib::lookup(sm, imported) {
                        Some(b) => self.define(local, *span, b),
                        None => self.err_help(
                            code::UNKNOWN_EXPORT,
                            *span,
                            format!("`{imported}` is not exported by `{}`", imp.source),
                            "see the typings in .plinth/types for the available names",
                        ),
                    }
                }
            }
            ModRef::User(dep) => {
                if let Some((local, span)) = &imp.default {
                    match self.defaults[dep].clone() {
                        Some(b) => self.define(local, *span, b),
                        None => self.err(code::UNKNOWN_EXPORT, *span, format!("`{}` has no default export", imp.source)),
                    }
                }
                for (imported, local, span) in &imp.names {
                    match self.exports[dep].get(imported).cloned() {
                        Some(b) => self.define(local, *span, b),
                        None => self.err(
                            code::UNKNOWN_EXPORT,
                            *span,
                            format!("`{}` does not export `{imported}`", imp.source),
                        ),
                    }
                }
            }
        }
    }

    fn declare_enum(&mut self, e: &ast::EnumDecl) {
        let mut members = Vec::new();
        let mut next = 0.0;
        for (name, value, span) in &e.members {
            let v = value.unwrap_or(next);
            if v.fract() != 0.0 || v < i32::MIN as f64 || v > i32::MAX as f64 {
                self.err(code::UNSUPPORTED, *span, "enum values must be integers");
            }
            members.push((name.clone(), v as i32));
            next = v + 1.0;
        }
        let id = self.prog.enums.len() as types::EnumId;
        self.prog.enums.push(EnumDef { name: e.name.clone(), members });
        self.define(&e.name, e.span, Binding::Enum(id));
        if e.exported {
            self.exports[self.module].insert(e.name.clone(), Binding::Enum(id));
        }
    }

    // -- Scopes -----------------------------------------------------------

    /// Defines a name in the innermost scope (or the module scope).
    fn define(&mut self, name: &str, span: Span, b: Binding) {
        let map = match self.fx.scopes.last_mut() {
            Some(s) => &mut s.names,
            None => &mut self.module_scopes[self.module],
        };
        if map.contains_key(name) {
            self.diags.push(Diagnostic::error(code::DUPLICATE, span, format!("`{name}` is already defined in this scope")));
            return;
        }
        map.insert(name.to_owned(), b);
    }

    fn lookup(&self, name: &str) -> Option<Binding> {
        for s in self.fx.scopes.iter().rev() {
            if let Some(b) = s.names.get(name) {
                return Some(b.clone());
            }
        }
        match self.module_scopes[self.module].get(name) {
            Some(b) => Some(b.clone()),
            None if name == "Error" => Some(Binding::Type(Type::Struct(self.error_class))),
            None => None,
        }
    }

    /// Declares the built-in `Error` class from `ERROR_PRELUDE` (SPEC.md
    /// §5.6). Codegen emits its constructor only if the app uses it.
    fn declare_error_class(&mut self, file: FileId) {
        let mut diags = Vec::new();
        let ast = crate::parse::parse(file, ERROR_PRELUDE, false, &mut diags);
        assert!(diags.is_empty(), "the Error prelude does not parse: {diags:?}");
        let Some(Item::Class(class)) = ast.items.first() else { unreachable!("the Error prelude is one class") };
        let id = self.prog.structs.len() as types::StructId;
        self.prog.structs.push(StructDef { name: "Error".into(), fields: Vec::new() });
        self.error_class = id;
        self.prog.error_class = Some(id);
        self.declare_class(id, class, 0);
    }

    /// The `Error` type (SPEC.md §5.6).
    pub(crate) fn error_type(&self) -> Type {
        Type::Struct(self.error_class)
    }

    /// `new Error(message)` for a string `message`.
    pub(crate) fn new_error(&mut self, message: TExpr) -> TExpr {
        let span = message.span;
        let ctor = self.classes[&self.error_class].ctor.expect("the Error prelude has a constructor");
        let arg = self.coerce(message, &Type::String.nullable());
        TExpr::new(TExprKind::Call(ctor, vec![arg]), self.error_type(), span)
    }

    /// True if `t` is `Error` or one of its subclasses.
    pub(crate) fn is_error_class(&self, t: &Type) -> bool {
        matches!(t, Type::Struct(sid) if self.is_subclass(*sid, self.error_class))
    }

    fn narrowed(&self, key: &NarrowKey) -> Option<Type> {
        for s in self.fx.scopes.iter().rev() {
            if let Some(t) = s.narrow.get(key) {
                return Some(t.clone());
            }
        }
        None
    }

    /// Drops all member-path narrowing (`r.subtitle`, not plain `x`) in
    /// every live scope: called after anything that could change a field
    /// through an alias (an assignment to any member path, or a call).
    /// Plain variable narrowing never needs this: it only ever narrows a
    /// `const`/parameter, which cannot be reassigned.
    pub(crate) fn invalidate_member_narrowing(&mut self) {
        for s in self.fx.scopes.iter_mut() {
            s.narrow.retain(|k, _| k.1.is_empty());
        }
    }

    fn push_scope(&mut self) {
        self.fx.scopes.push(Scope::default());
    }

    fn pop_scope(&mut self) {
        self.fx.scopes.pop();
    }

    fn new_var(&mut self, name: &str, ty: Type, mutable: bool) -> VarId {
        let module = if self.fx.scopes.is_empty() && matches!(self.prog.funcs[self.fx.func as usize].kind, FuncKind::ModuleInit(_))
        {
            Some(self.module as u32)
        } else {
            None
        };
        self.prog.new_var(VarInfo {
            name: name.to_owned(),
            ty,
            owner: self.fx.func,
            mutable,
            module,
            in_loop: self.fx.loops.last().copied(),
            captured: false,
        })
    }

    /// A compiler temporary in the current function.
    fn temp(&mut self, ty: Type) -> VarId {
        let n = self.prog.vars.len();
        self.prog.new_var(VarInfo {
            name: format!("$t{n}"),
            ty,
            owner: self.fx.func,
            mutable: true,
            module: None,
            in_loop: self.fx.loops.last().copied(),
            captured: false,
        })
    }

    // -- Types ------------------------------------------------------------

    fn fields(&mut self, anns: &[ast::FieldAnn]) -> Vec<Field> {
        let mut fields: Vec<Field> = Vec::new();
        for f in anns {
            if fields.iter().any(|x| x.name == f.name) {
                self.err(code::DUPLICATE, f.span, format!("duplicate field `{}`", f.name));
                continue;
            }
            let mut ty = self.resolve_type(&f.ty);
            if f.optional {
                ty = self.nullable(ty, f.span);
            }
            fields.push(Field { name: f.name.clone(), ty, optional: f.optional });
        }
        fields
    }

    /// `T | null`, with a check that the representation supports null.
    /// `number`, `int`, `boolean` and enums are boxed (HANDOFF.md item 2);
    /// everything else of `Ref` representation reuses its own 0-is-null.
    fn nullable(&mut self, ty: Type, span: Span) -> Type {
        match ty {
            Type::Signal(_) | Type::Computed(_) => {
                self.err_help(
                    code::NULLABLE,
                    span,
                    format!("`{} | null` is not supported yet", self.show(&ty)),
                    "use a default value instead of null",
                );
                Type::Error
            }
            Type::Void => Type::Null,
            t => t.nullable(),
        }
    }

    pub fn resolve_type(&mut self, ann: &TypeAnn) -> Type {
        match ann {
            TypeAnn::Number(_) | TypeAnn::NumLit(..) => Type::Number,
            TypeAnn::String(_) => Type::String,
            TypeAnn::Boolean(_) | TypeAnn::BoolLit(..) => Type::Bool,
            TypeAnn::Void(_) => Type::Void,
            TypeAnn::Null(_) => Type::Null,
            TypeAnn::StrLit(s, _) => Type::str_lits(vec![s.clone()]),
            TypeAnn::Array(t, _) => Type::Array(Box::new(self.resolve_type(t))),
            TypeAnn::Union(parts, span) => {
                let mut has_null = false;
                let mut lits = Vec::new();
                let mut others = Vec::new();
                for p in parts {
                    match p {
                        TypeAnn::Null(_) => has_null = true,
                        TypeAnn::StrLit(s, _) => lits.push(s.clone()),
                        other => others.push(self.resolve_type(other)),
                    }
                }
                // Number and boolean literal unions widen to their base type.
                others.dedup();
                let base = match (lits.is_empty(), others.len()) {
                    (false, 0) => Type::str_lits(lits),
                    (true, 1) => others.pop().unwrap(),
                    (true, 0) => Type::Null,
                    _ => {
                        if others.iter().all(|t| *t == Type::Number) && lits.is_empty() {
                            Type::Number
                        } else if lits.is_empty() {
                            self.union_of(others, *span)
                        } else {
                            self.err_help(
                                code::ADVANCED_TYPE,
                                *span,
                                "a union cannot mix string literals with other types",
                                "use `T | null`, or a union of string literals",
                            );
                            return Type::Error;
                        }
                    }
                };
                if has_null && base != Type::Null { self.nullable(base, *span) } else { base }
            }
            TypeAnn::Func { params, ret, .. } => {
                let mut ps = Vec::new();
                let mut required = 0;
                for (i, (_, t, optional)) in params.iter().enumerate() {
                    let mut ty = self.resolve_type(t);
                    if *optional {
                        ty = self.nullable(ty, t.span());
                    } else {
                        required = i + 1;
                    }
                    ps.push(ty);
                }
                Type::Func(Rc::new(FuncType { params: ps, required, ret: self.resolve_type(ret) }))
            }
            TypeAnn::Object(fields, _) => {
                let fields = self.fields(fields);
                self.anon_struct(fields)
            }
            TypeAnn::Tuple(parts, _) => self.tuple_type(parts),
            TypeAnn::Named { name, args, span } => self.named_type(name, args, *span),
        }
    }

    fn named_type(&mut self, name: &str, args: &[TypeAnn], span: Span) -> Type {
        if args.is_empty() {
            if let Some(t) = self.generic_bindings.get(name) {
                return t.clone();
            }
        }
        let arity = |c: &mut Self, n: usize| {
            if args.len() != n {
                c.err(code::UNKNOWN_TYPE, span, format!("`{name}` takes {n} type argument(s)"));
                false
            } else {
                true
            }
        };
        match name {
            "Array" | "ReadonlyArray" => {
                if !arity(self, 1) {
                    return Type::Error;
                }
                return Type::Array(Box::new(self.resolve_type(&args[0])));
            }
            "JSX.Element" => return Type::Element,
            "int" => return Type::Int,
            "any" | "unknown" | "object" | "Object" => {
                self.err(code::ANY, span, format!("`{name}` is not allowed"));
                return Type::Error;
            }
            "Map" => {
                if !arity(self, 2) {
                    return Type::Error;
                }
                let k = self.resolve_type(&args[0]);
                let v = self.resolve_type(&args[1]);
                if !self.valid_key_type(&k) {
                    let msg = format!("a `Map` key must be `string`, `int`, `number`, `boolean` or an enum, not `{}`", self.show(&k));
                    self.err(code::ADVANCED_TYPE, span, msg);
                    return Type::Error;
                }
                if !self.valid_map_value_type(&v) {
                    let msg = format!("a `Map`/`Set` value of type `{}` is not supported yet", self.show(&v));
                    self.err_help(code::ADVANCED_TYPE, span, msg, "use `string`, `number` or an object type");
                    return Type::Error;
                }
                return Type::Map(Box::new(k), Box::new(v));
            }
            "Set" => {
                if !arity(self, 1) {
                    return Type::Error;
                }
                let t = self.resolve_type(&args[0]);
                if !self.valid_key_type(&t) {
                    let msg = format!("a `Set` element must be `string`, `int`, `number`, `boolean` or an enum, not `{}`", self.show(&t));
                    self.err(code::ADVANCED_TYPE, span, msg);
                    return Type::Error;
                }
                return Type::Set(Box::new(t));
            }
            "Promise" => {
                if !arity(self, 1) {
                    return Type::Error;
                }
                let t = self.resolve_type(&args[0]);
                if t.is_error() {
                    return Type::Error;
                }
                return self.promise_type(&t);
            }
            "PromiseSettledResult" | "PromiseFulfilledResult" if self.lookup(name).is_none() => {
                if !arity(self, 1) {
                    return Type::Error;
                }
                let t = self.resolve_type(&args[0]);
                if t.is_error() {
                    return Type::Error;
                }
                let (ok, _, u) = self.settled_result(&t);
                return if name == "PromiseFulfilledResult" { Type::Struct(ok) } else { u };
            }
            "PromiseRejectedResult" if self.lookup(name).is_none() => {
                if !arity(self, 0) {
                    return Type::Error;
                }
                let (_, bad, _) = self.settled_result(&Type::Void);
                return Type::Struct(bad);
            }
            "Record" | "Partial" | "Readonly" => {
                self.err(code::ADVANCED_TYPE, span, format!("`{name}` is not supported yet"));
                return Type::Error;
            }
            _ => {}
        }
        match self.lookup(name) {
            Some(Binding::Type(Type::Signal(_))) | Some(Binding::Type(Type::Computed(_))) => {
                if !arity(self, 1) {
                    return Type::Error;
                }
                let inner = self.resolve_type(&args[0]);
                match self.lookup(name) {
                    Some(Binding::Type(Type::Signal(_))) => Type::Signal(Box::new(inner)),
                    _ => Type::Computed(Box::new(inner)),
                }
            }
            Some(Binding::Type(t)) => {
                if !args.is_empty() {
                    self.err(code::GENERIC_USER, span, format!("`{name}` is not generic"));
                }
                t
            }
            Some(Binding::Alias(m, idx)) => self.resolve_alias(m, idx, args, span),
            Some(Binding::Interface(m, idx)) => self.resolve_interface(m, idx, args, span),
            Some(Binding::Enum(e)) => Type::Enum(e),
            _ => {
                self.err(code::UNKNOWN_TYPE, span, format!("unknown type `{name}`"));
                Type::Error
            }
        }
    }

    fn resolve_alias(&mut self, m: usize, idx: usize, args: &[TypeAnn], span: Span) -> Type {
        let type_params = self.aliases[m][idx].type_params.clone();
        if type_params.is_empty() {
            if !args.is_empty() {
                self.err(code::GENERIC_USER, span, format!("`{}` is not generic", self.aliases[m][idx].name));
            }
            return self.resolve_alias_body(m, idx, span);
        }
        if args.len() != type_params.len() {
            self.err(code::ARG_COUNT, span, format!("`{}` takes {} type argument(s)", self.aliases[m][idx].name, type_params.len()));
            return Type::Error;
        }
        // Type arguments resolve in the *caller's* scope, before switching
        // to the alias's own module/bindings.
        let bound: Vec<Type> = args.iter().map(|a| self.resolve_type(a)).collect();
        if bound.iter().any(Type::is_error) {
            return Type::Error;
        }
        if let Some((_, _, _, t)) = self.alias_instantiations.iter().find(|(am, ai, b, _)| *am == m && *ai == idx && *b == bound) {
            return t.clone();
        }
        let saved_bindings = std::mem::take(&mut self.generic_bindings);
        for (n, t) in type_params.iter().zip(&bound) {
            self.generic_bindings.insert(n.clone(), t.clone());
        }
        let t = self.resolve_alias_body(m, idx, span);
        self.generic_bindings = saved_bindings;
        self.alias_instantiations.push((m, idx, bound, t.clone()));
        t
    }

    /// Resolves a (possibly generic) alias's body annotation, with
    /// `self.generic_bindings` already set for a generic alias.
    fn resolve_alias_body(&mut self, m: usize, idx: usize, span: Span) -> Type {
        if !self.resolving_alias.insert((m, idx)) {
            self.err(code::ADVANCED_TYPE, span, "recursive type aliases are not supported");
            return Type::Error;
        }
        let ann = self.aliases[m][idx].ty.clone();
        // The alias body resolves in its own module's scope.
        let saved = std::mem::replace(&mut self.module, m);
        let saved_scopes = std::mem::take(&mut self.fx.scopes);
        let t = self.resolve_type(&ann);
        self.fx.scopes = saved_scopes;
        self.module = saved;
        self.resolving_alias.remove(&(m, idx));
        t
    }

    /// Monomorphizes a generic interface for one set of type arguments,
    /// reusing an earlier instantiation with the same arguments (like
    /// `resolve_alias`, but each distinct instantiation is its own nominal
    /// struct, not a structurally-deduped anonymous one).
    fn resolve_interface(&mut self, m: usize, idx: usize, args: &[TypeAnn], span: Span) -> Type {
        let type_params = self.interfaces[m][idx].type_params.clone();
        if args.len() != type_params.len() {
            self.err(code::ARG_COUNT, span, format!("`{}` takes {} type argument(s)", self.interfaces[m][idx].name, type_params.len()));
            return Type::Error;
        }
        // Type arguments resolve in the *caller's* scope, before switching
        // to the interface's own module/bindings.
        let bound: Vec<Type> = args.iter().map(|a| self.resolve_type(a)).collect();
        if bound.iter().any(Type::is_error) {
            return Type::Error;
        }
        if let Some((_, _, _, id)) = self.interface_instantiations.iter().find(|(im, ii, b, _)| *im == m && *ii == idx && *b == bound) {
            return Type::Struct(*id);
        }
        if !self.resolving_interface.insert((m, idx)) {
            self.err(code::ADVANCED_TYPE, span, "recursive generic interfaces are not supported");
            return Type::Error;
        }
        let name = format!("{}<{}>", self.interfaces[m][idx].name, bound.iter().map(|t| self.show(t)).collect::<Vec<_>>().join(", "));
        let id = self.prog.structs.len() as types::StructId;
        self.prog.structs.push(StructDef { name, fields: Vec::new() });
        let saved_bindings = std::mem::take(&mut self.generic_bindings);
        for (n, t) in type_params.iter().zip(&bound) {
            self.generic_bindings.insert(n.clone(), t.clone());
        }
        let field_anns = self.interfaces[m][idx].fields.clone();
        let saved_module = std::mem::replace(&mut self.module, m);
        let saved_scopes = std::mem::take(&mut self.fx.scopes);
        let fields = self.fields(&field_anns);
        self.fx.scopes = saved_scopes;
        self.module = saved_module;
        self.prog.structs[id as usize].fields = fields;
        self.generic_bindings = saved_bindings;
        self.resolving_interface.remove(&(m, idx));
        self.interface_instantiations.push((m, idx, bound, id));
        Type::Struct(id)
    }

    /// A `Map` key or `Set` element type (SPEC.md §4.2).
    fn valid_key_type(&self, t: &Type) -> bool {
        matches!(t, Type::String | Type::StrLits(_) | Type::Number | Type::Int | Type::Bool | Type::Enum(_))
    }

    /// A `Map` value type. `get` needs a nullable result, so this is every
    /// type `nullable` accepts (HANDOFF.md item 2 lifted the v0
    /// restriction against `boolean`/`int`/enum, boxed the same way).
    fn valid_map_value_type(&self, t: &Type) -> bool {
        matches!(t, Type::String | Type::StrLits(_) | Type::Number | Type::Struct(_) | Type::Array(_) | Type::Union(_))
            || matches!(t, Type::Bool | Type::Int | Type::Enum(_))
    }

    /// The comparison for a `Map` key or `Set` element.
    pub(crate) fn key_eq(&self, k: &Type) -> EqKind {
        match self.widen(k.clone()) {
            Type::String => EqKind::Str,
            Type::Number => EqKind::F64,
            _ => EqKind::I32,
        }
    }

    /// The internal 2-field struct `{ keys: K[], values: V[] }` that backs
    /// a `Map<K, V>` at run time (HANDOFF.md item 5). A `Map`'s own fields
    /// are never exposed to user code; only `Checker` methods read them.
    pub(crate) fn map_struct(&mut self, k: &Type, v: &Type) -> types::StructId {
        let fields = vec![
            Field { name: "keys".into(), ty: Type::Array(Box::new(k.clone())), optional: false },
            Field { name: "values".into(), ty: Type::Array(Box::new(v.clone())), optional: false },
        ];
        match self.anon_struct(fields) {
            Type::Struct(s) => s,
            _ => unreachable!(),
        }
    }

    /// The internal 1-field struct `{ keys: T[] }` that backs a `Set<T>`.
    pub(crate) fn set_struct(&mut self, t: &Type) -> types::StructId {
        let fields = vec![Field { name: "keys".into(), ty: Type::Array(Box::new(t.clone())), optional: false }];
        match self.anon_struct(fields) {
            Type::Struct(s) => s,
            _ => unreachable!(),
        }
    }

    /// The struct that backs a tuple `[A, B, …]`: one field per element,
    /// named `0`, `1`, …, and the struct name is the tuple type as written
    /// (for diagnostics). `Map.entries()` returns an array of these.
    pub(crate) fn tuple_struct(&mut self, elems: &[Type]) -> types::StructId {
        let shown: Vec<String> = elems.iter().map(|t| self.show(t)).collect();
        let name = format!("[{}]", shown.join(", "));
        let fields = elems.iter().enumerate().map(|(i, t)| Field { name: i.to_string(), ty: t.clone(), optional: false }).collect();
        self.tuple_struct_named(name, fields)
    }

    fn tuple_struct_named(&mut self, name: String, fields: Vec<Field>) -> types::StructId {
        if let Some(id) = self.anon_structs.get(&name) {
            return *id;
        }
        let id = self.prog.structs.len() as types::StructId;
        self.prog.structs.push(StructDef { name: name.clone(), fields });
        self.anon_structs.insert(name, id);
        id
    }

    /// A tuple type `[A, B?, ...C[]]`. An optional element is a field of
    /// type `T | null` (marked `optional`); a rest element is a last field
    /// named `...` that holds an array. `[...T[]]` is `T[]`.
    fn tuple_type(&mut self, parts: &[(TypeAnn, ast::TupleMark)]) -> Type {
        let mut fields = Vec::new();
        let mut shown = Vec::new();
        let mut rest = None;
        for (p, mark) in parts {
            let t = self.resolve_type(p);
            if t.is_error() {
                return Type::Error;
            }
            let s = self.show(&t);
            let s = if s.contains(' ') { format!("({s})") } else { s };
            match mark {
                ast::TupleMark::Required => {
                    shown.push(s);
                    fields.push(Field { name: fields.len().to_string(), ty: t, optional: false });
                }
                ast::TupleMark::Optional => {
                    shown.push(format!("{s}?"));
                    let t = self.nullable(t, p.span());
                    if t.is_error() {
                        return Type::Error;
                    }
                    fields.push(Field { name: fields.len().to_string(), ty: t, optional: true });
                }
                ast::TupleMark::Rest => {
                    if !matches!(t, Type::Array(_)) {
                        let msg = format!("a rest element must have an array type, not `{}`", self.show(&t));
                        self.err_help(code::ADVANCED_TYPE, p.span(), msg, "write `...T[]`, for example `[string, ...number[]]`");
                        return Type::Error;
                    }
                    shown.push(format!("...{}", self.show(&t)));
                    rest = Some(t);
                }
            }
        }
        if fields.is_empty()
            && let Some(r) = rest
        {
            return r;
        }
        if let Some(r) = rest {
            fields.push(Field { name: "...".into(), ty: r, optional: false });
        }
        Type::Struct(self.tuple_struct_named(format!("[{}]", shown.join(", ")), fields))
    }

    /// The shape of a tuple struct, or `None` for any other struct.
    pub(crate) fn tuple_shape(&self, sid: types::StructId) -> Option<TupleShape> {
        let def = &self.prog.structs[sid as usize];
        if !def.name.starts_with('[') {
            return None;
        }
        let mut shape = TupleShape { fixed: Vec::new(), required: 0, rest: None };
        for f in &def.fields {
            match &f.ty {
                Type::Array(e) if f.name == "..." => shape.rest = Some((**e).clone()),
                t => {
                    if !f.optional {
                        shape.required += 1;
                    }
                    shape.fixed.push(t.clone());
                }
            }
        }
        Some(shape)
    }

    /// `ChartPoint` from `plinth:ui`: `{ label: string; value: number }`,
    /// the same struct an object literal of this shape gets.
    fn chart_point_type(&mut self) -> Type {
        self.anon_struct(vec![
            Field { name: "label".into(), ty: Type::String, optional: false },
            Field { name: "value".into(), ty: Type::Number, optional: false },
        ])
    }

    fn anon_struct(&mut self, fields: Vec<Field>) -> Type {
        let key: Vec<String> = fields.iter().map(|f| format!("{}:{}", f.name, self.show(&f.ty))).collect();
        let key = key.join(",");
        if let Some(id) = self.anon_structs.get(&key) {
            return Type::Struct(*id);
        }
        let id = self.prog.structs.len() as types::StructId;
        let name = format!("{{ {} }}", key.replace(',', "; ").replace(':', ": "));
        self.prog.structs.push(StructDef { name, fields });
        self.anon_structs.insert(key, id);
        Type::Struct(id)
    }

    /// Builds a `Type::Union` from distinct object and string member
    /// types (SPEC.md §4.2). Members with another representation (number,
    /// boolean, enum, array, function, …) are not supported yet.
    fn union_of(&mut self, mut members: Vec<Type>, span: Span) -> Type {
        members.dedup();
        let mut strings = 0;
        for m in &members {
            match m {
                Type::Struct(_) => {}
                Type::String | Type::StrLits(_) => strings += 1,
                other => {
                    let msg = format!("a union member of type `{}` is not supported yet", self.show(other));
                    self.err_help(code::ADVANCED_TYPE, span, msg, "union members can be object types or `string`");
                    return Type::Error;
                }
            }
        }
        if strings > 1 {
            self.err(code::ADVANCED_TYPE, span, "a union can have at most one string-like member");
            return Type::Error;
        }
        if members.len() < 2 {
            return members.pop().unwrap_or(Type::Error);
        }
        Type::Union(members.into())
    }

    /// If the object (struct) members of a union all share a literal-string
    /// field, declared first, with a distinct literal per member: the
    /// field's name and `(literal, member index)` pairs (indices into the
    /// full `members`, including any non-struct member). A union needs at
    /// least one struct member for this. This field is readable without
    /// narrowing, and narrows the union when compared to one of its
    /// literals (HANDOFF.md item 2).
    pub(crate) fn union_discriminant(&self, members: &[Type]) -> Option<(String, Vec<(String, usize)>)> {
        let mut name: Option<String> = None;
        let mut out = Vec::new();
        let mut any_struct = false;
        for (i, m) in members.iter().enumerate() {
            let Type::Struct(sid) = m else { continue };
            any_struct = true;
            let f = self.prog.structs[*sid as usize].fields.first()?;
            let Type::StrLits(lits) = &f.ty else { return None };
            if lits.len() != 1 {
                return None;
            }
            match &name {
                None => name = Some(f.name.clone()),
                Some(n) if *n != f.name => return None,
                _ => {}
            }
            out.push((lits[0].clone(), i));
        }
        if !any_struct {
            return None;
        }
        let mut seen = std::collections::HashSet::new();
        if !out.iter().all(|(l, _)| seen.insert(l.clone())) {
            return None;
        }
        Some((name?, out))
    }

    // -- Assignability ----------------------------------------------------

    /// How a value of `from` converts to `to`: `None` if it cannot,
    /// `Some(None)` if no code is needed.
    fn conversion(&self, from: &Type, to: &Type) -> Option<Option<Coercion>> {
        use Type as T;
        if from == to {
            return Some(None);
        }
        match (from, to) {
            (T::Error, _) | (_, T::Error) => Some(None),
            (T::Int, T::Number) => Some(Some(Coercion::I32ToNum)),
            (T::StrLits(_), T::String) => Some(Some(Coercion::Retag)),
            (T::StrLits(a), T::StrLits(b)) if a.iter().all(|x| b.contains(x)) => Some(Some(Coercion::Retag)),
            (T::Null, T::Nullable(_)) | (T::Null, T::Element) => Some(Some(Coercion::Retag)),
            (T::Number, T::Nullable(inner)) if **inner == T::Number => Some(Some(Coercion::BoxNum)),
            (t, T::Nullable(inner)) if *t == **inner && t.repr() == types::Repr::I32 => Some(Some(Coercion::BoxI32)),
            (t, T::Nullable(inner)) if t.repr() == types::Repr::Ref => {
                self.conversion(t, inner).filter(|c| c.is_none_or(|c| c == Coercion::Retag)).map(|_| Some(Coercion::Retag))
            }
            (T::Nullable(a), T::Nullable(b)) => {
                self.conversion(a, b).filter(|c| c.is_none_or(|c| c == Coercion::Retag)).map(|_| Some(Coercion::Retag))
            }
            // A subclass value is a no-op upcast to a base class (or
            // itself): the base's fields are a byte-identical prefix of the
            // subclass's layout (SPEC.md §4.2 v1).
            (T::Struct(a), T::Struct(b)) if self.is_subclass(*a, *b) => Some(Some(Coercion::Retag)),
            (T::Struct(a), T::Struct(b)) => self.same_layout(*a, *b).then_some(Some(Coercion::Retag)),
            (T::Array(a), T::Array(b)) => {
                (a.repr() == b.repr() && self.conversion(a, b).is_some_and(|c| c.is_none_or(|c| c == Coercion::Retag)))
                    .then_some(Some(Coercion::Retag))
            }
            (T::Func(f), T::Func(g)) => self.func_compatible(f, g).then_some(Some(Coercion::Retag)),
            (t, T::Union(members)) if members.iter().any(|m| m == t) => Some(Some(Coercion::Retag)),
            (T::StrLits(_), T::Union(members)) if members.contains(&T::String) => Some(Some(Coercion::Retag)),
            _ => None,
        }
    }

    /// True if `sub` is `base` or a (transitive) subclass of it.
    fn is_subclass(&self, sub: types::StructId, base: types::StructId) -> bool {
        let mut cur = Some(sub);
        while let Some(s) = cur {
            if s == base {
                return true;
            }
            cur = self.classes.get(&s).and_then(|c| c.base);
        }
        false
    }

    /// The method named `name` reachable from class `sid`: its own, or the
    /// nearest base class's (SPEC.md §4.2 v1 overriding).
    fn resolve_method(&self, sid: types::StructId, name: &str) -> Option<FuncId> {
        let mut cur = Some(sid);
        while let Some(s) = cur {
            let info = self.classes.get(&s)?;
            if let Some(&fid) = info.methods.get(name) {
                return Some(fid);
            }
            cur = info.base;
        }
        None
    }

    fn same_layout(&self, a: types::StructId, b: types::StructId) -> bool {
        let (fa, fb) = (&self.prog.structs[a as usize].fields, &self.prog.structs[b as usize].fields);
        fa.len() == fb.len() && fa.iter().zip(fb).all(|(x, y)| x.name == y.name && x.ty.repr() == y.ty.repr() && self.conversion(&x.ty, &y.ty).is_some())
    }

    fn func_compatible(&self, f: &FuncType, g: &FuncType) -> bool {
        f.params.len() == g.params.len()
            && f.params.iter().zip(&g.params).all(|(a, b)| a.repr() == b.repr() && self.conversion(b, a).is_some())
            && (g.ret == Type::Void || (f.ret.repr() == g.ret.repr() && self.conversion(&f.ret, &g.ret).is_some()))
            && (f.ret.repr() == g.ret.repr() || g.ret == Type::Void && f.ret == Type::Void)
    }

    /// Converts `e` to `to`, or reports a mismatch.
    fn coerce(&mut self, e: TExpr, to: &Type) -> TExpr {
        match self.conversion(&e.ty, to) {
            Some(None) => TExpr { ty: if e.ty.is_error() { e.ty } else { to.clone() }, ..e },
            Some(Some(c)) => {
                let span = e.span;
                TExpr::new(TExprKind::Coerce(c, Box::new(e)), to.clone(), span)
            }
            None => {
                let msg = format!("type `{}` is not assignable to `{}`", self.show(&e.ty), self.show(to));
                let help = match (&e.ty, to) {
                    (Type::Nullable(_), _) => Some("check for null first, or use `??` to give a default"),
                    (Type::Struct(_), Type::Struct(_)) => {
                        Some("objects convert only when the fields are the same, in the same order (itables come in v1)")
                    }
                    (Type::Number, Type::String) => Some("use a template literal: `${n}`"),
                    _ => None,
                };
                match help {
                    Some(h) => self.err_help(code::TYPE_MISMATCH, e.span, msg, h),
                    None => self.err(code::TYPE_MISMATCH, e.span, msg),
                }
                TExpr { ty: Type::Error, ..e }
            }
        }
    }

    /// Converts any value to a string (templates, `+` and text children).
    fn to_str(&mut self, e: TExpr) -> TExpr {
        let span = e.span;
        let c = match &e.ty {
            Type::String | Type::Error => return e,
            Type::StrLits(_) => Coercion::Retag,
            Type::Number => Coercion::NumToStr,
            Type::Bool => Coercion::BoolToStr,
            Type::Enum(_) | Type::Int => {
                let n = TExpr::new(TExprKind::Coerce(Coercion::I32ToNum, Box::new(e)), Type::Number, span);
                return TExpr::new(TExprKind::Coerce(Coercion::NumToStr, Box::new(n)), Type::String, span);
            }
            _ => {
                let msg = format!("a value of type `{}` cannot become a string", self.show(&e.ty));
                self.err(code::TYPE_MISMATCH, span, msg);
                return TExpr { ty: Type::Error, ..e };
            }
        };
        TExpr::new(TExprKind::Coerce(c, Box::new(e)), Type::String, span)
    }

    fn truthy(&mut self, e: TExpr) -> TExpr {
        if e.ty == Type::Bool {
            return e;
        }
        match e.ty {
            Type::Void | Type::Func(_) | Type::Signal(_) | Type::Computed(_) => {
                let msg = format!("a value of type `{}` cannot be a condition", self.show(&e.ty));
                let help = if matches!(e.ty, Type::Signal(_) | Type::Computed(_)) { "call it to read its value" } else { "" };
                if help.is_empty() {
                    self.err(code::TYPE_MISMATCH, e.span, msg);
                } else {
                    self.err_help(code::TYPE_MISMATCH, e.span, msg, help);
                }
            }
            _ => {}
        }
        let span = e.span;
        TExpr::new(TExprKind::Coerce(Coercion::Truthy, Box::new(e)), Type::Bool, span)
    }

    /// Removes literal types for variable declarations (`let x = "a"`).
    fn widen(&self, t: Type) -> Type {
        match t {
            Type::StrLits(_) => Type::String,
            t => t,
        }
    }

    // -- Functions --------------------------------------------------------

    fn declare_top_func(&mut self, f: &ast::FuncDecl, m: usize) {
        let name = f.name.as_ref().map(|n| n.0.clone()).unwrap_or_else(|| "default".into());
        let fid = self.prog.new_func(FuncDef {
            name: name.clone(),
            kind: FuncKind::TopLevel,
            params: Vec::new(),
            ret: Type::Error,
            body: Vec::new(),
            span: f.span,
        });
        // Parameters belong to the function itself.
        let saved = std::mem::replace(&mut self.fx.func, fid);
        let mut params = Vec::new();
        for p in &f.params {
            let ty = match (&p.ty, &p.default) {
                (Some(t), _) => {
                    let ty = self.resolve_type(t);
                    if p.optional { self.nullable(ty, p.span) } else { ty }
                }
                (None, Some(d)) => match &d.kind {
                    ast::ExprKind::Num(_) => Type::Number,
                    ast::ExprKind::Str(_) => Type::String,
                    ast::ExprKind::Bool(_) => Type::Bool,
                    _ => Type::Error,
                },
                (None, None) => {
                    self.err_help(code::CANNOT_INFER, p.span, "a parameter of a top-level function needs a type", "add `: type`");
                    Type::Error
                }
            };
            let pname = match &p.pattern {
                ast::Pattern::Ident(n, _) => n.clone(),
                _ => format!("$p{}", params.len()),
            };
            params.push(self.prog.new_var(VarInfo {
                name: pname,
                ty,
                owner: fid,
                mutable: true,
                module: None,
                in_loop: None,
                captured: false,
            }));
        }
        self.fx.func = saved;
        self.prog.funcs[fid as usize].params = params;
        if let Some(r) = &f.ret {
            let ret = self.resolve_type(r);
            self.prog.funcs[fid as usize].ret = ret;
        }
        let fidx = fid;
        if f.is_default {
            self.defaults[m] = Some(Binding::Func(fidx));
        }
        if let Some((n, span)) = &f.name {
            self.define(n, *span, Binding::Func(fidx));
            if f.exported && !f.is_default {
                self.exports[m].insert(name, Binding::Func(fidx));
            }
        }
        self.pending.insert(fid, PendingFunc { decl: f.clone(), module: m });
    }

    // -- Classes (SPEC.md §4.2) --------------------------------------------

    /// Fills in a class's fields, lowers its methods to top-level functions
    /// (a synthetic `this: ClassName` first parameter, checked lazily like
    /// any other top-level function), and checks its constructor eagerly
    /// (so it can bind `this` and build the instance; as a result a
    /// constructor cannot call a free function declared later in the same
    /// file — forward-declare it, or move the class after it).
    fn declare_class(&mut self, sid: types::StructId, c: &ast::ClassDecl, m: usize) {
        // -- Base class (SPEC.md §4.2 v1: single inheritance) -------------
        let base = match &c.extends {
            None => None,
            Some((name, espan)) => match self.lookup(name) {
                Some(Binding::Type(Type::Struct(bsid))) => match self.classes.get(&bsid) {
                    Some(_) => Some(bsid),
                    None => {
                        self.err_help(
                            code::EXTENDS,
                            *espan,
                            format!("`{name}` is not a fully declared class yet"),
                            "a base class must be declared before its subclass; this also catches a cycle",
                        );
                        None
                    }
                },
                Some(Binding::Type(_)) => {
                    self.err_help(code::EXTENDS, *espan, format!("`{name}` is not a class"), "a class can only extend another class");
                    None
                }
                Some(_) => {
                    self.err(code::EXTENDS, *espan, format!("`{name}` is not a class"));
                    None
                }
                None => {
                    self.err(code::UNKNOWN_NAME, *espan, format!("cannot find name `{name}`"));
                    None
                }
            },
        };
        let base_fields: Vec<Field> = match base {
            Some(bsid) => self.prog.structs[bsid as usize].fields.clone(),
            None => Vec::new(),
        };
        let base_field_count = base_fields.len();

        let mut fields: Vec<Field> = base_fields;
        let mut inits: Vec<Option<ast::Expr>> = vec![None; base_field_count];
        for f in &c.fields {
            if fields.iter().any(|x| x.name == f.name) {
                self.err(code::DUPLICATE, f.span, format!("duplicate field `{}`", f.name));
                continue;
            }
            let ty = self.resolve_type(&f.ty);
            fields.push(Field { name: f.name.clone(), ty, optional: false });
            inits.push(f.init.clone());
        }
        self.prog.structs[sid as usize].fields = fields.clone();

        let mut methods = HashMap::new();
        for meth in &c.methods {
            let Some((name, name_span)) = meth.name.clone() else { continue };
            if methods.contains_key(&name) {
                self.err(code::DUPLICATE, name_span, format!("duplicate method `{name}`"));
                continue;
            }
            let mangled = format!("{}#{name}", c.name);
            let mut synth = meth.clone();
            synth.name = Some((mangled.clone(), name_span));
            synth.exported = false;
            synth.is_default = false;
            synth.params.insert(0, this_param(&c.name, meth.span));
            self.declare_top_func(&synth, m);
            let Some(Binding::Func(fid)) = self.module_scopes[m].get(&mangled).cloned() else { unreachable!() };
            if let Some(base_fid) = base.and_then(|b| self.resolve_method(b, &name)) {
                let bft = self.func_type(base_fid, name_span);
                let oft = self.func_type(fid, name_span);
                let b_rest = Rc::new(FuncType { params: bft.params[1..].to_vec(), required: bft.required.saturating_sub(1), ret: bft.ret.clone() });
                let o_rest = Rc::new(FuncType { params: oft.params[1..].to_vec(), required: oft.required.saturating_sub(1), ret: oft.ret.clone() });
                if !self.func_compatible(&o_rest, &b_rest) {
                    self.err_help(
                        code::OVERRIDE,
                        name_span,
                        format!("`{name}` does not override the base class's method with a compatible signature"),
                        "match the base method's parameter and return types",
                    );
                }
            }
            methods.insert(name, fid);
        }
        self.classes.insert(sid, ClassInfo { ctor: None, methods, base });

        // A class with no explicit constructor gets a trivial one that
        // builds the instance from the field initializers alone (every
        // field needs one, or a zero value, in that case). A subclass's
        // default constructor forwards to a zero-argument `super()`.
        let default_ctor = ast::CtorDecl {
            params: Vec::new(),
            body: if base.is_some() {
                vec![ast::Stmt {
                    kind: ast::StmtKind::Expr(ast::Expr {
                        kind: ast::ExprKind::Call {
                            callee: Box::new(ast::Expr { kind: ast::ExprKind::Ident("super".to_string()), span: c.span }),
                            type_args: Vec::new(),
                            args: Vec::new(),
                            optional: false,
                        },
                        span: c.span,
                    }),
                    span: c.span,
                }]
            } else {
                Vec::new()
            },
            span: c.span,
        };
        let ctor_ast = c.ctor.as_ref().unwrap_or(&default_ctor);
        if ctor_ast.body.iter().any(stmt_has_return) {
            self.err_help(
                code::CLASS,
                ctor_ast.span,
                "a constructor cannot `return` a value",
                "remove the `return`; the instance returns automatically",
            );
        }

        let fid = self.prog.new_func(FuncDef {
            name: format!("{}.constructor", c.name),
            kind: FuncKind::TopLevel,
            params: Vec::new(),
            ret: Type::Struct(sid),
            body: Vec::new(),
            span: ctor_ast.span,
        });
        let saved_func = std::mem::replace(&mut self.fx.func, fid);
        let mut param_vars = Vec::new();
        for p in &ctor_ast.params {
            let ty = match &p.ty {
                Some(t) => {
                    let ty = self.resolve_type(t);
                    if p.optional { self.nullable(ty, p.span) } else { ty }
                }
                None => {
                    self.err_help(code::CANNOT_INFER, p.span, "a constructor parameter needs a type", "add `: type`");
                    Type::Error
                }
            };
            let pname = match &p.pattern {
                ast::Pattern::Ident(n, _) => n.clone(),
                _ => format!("$p{}", param_vars.len()),
            };
            param_vars.push(self.prog.new_var(VarInfo {
                name: pname,
                ty,
                owner: fid,
                mutable: true,
                module: None,
                in_loop: None,
                captured: false,
            }));
        }
        self.fx.func = saved_func;
        self.prog.funcs[fid as usize].params = param_vars.clone();

        let saved_fx = std::mem::replace(
            &mut self.fx,
            FnCx { func: fid, scopes: vec![Scope::default()], loops: Vec::new(), ret: Some(Type::Struct(sid)), inferred: None, reactive: ReactiveCtx::Callback, is_async: false },
        );
        let mut prologue = Vec::new();
        for (p, v) in ctor_ast.params.iter().zip(&param_vars) {
            self.bind_param(p, *v, &mut prologue);
        }

        // A subclass constructor's first statement must be `super(...)`
        // (SPEC.md §4.2 v1): it runs the base class's constructor, and its
        // result supplies this class's inherited fields. Any other
        // `super(...)` call is rejected generically when the body is
        // checked (it is never valid there).
        let mut rest_body: &[ast::Stmt] = &ctor_ast.body;
        let mut base_field_reads: Vec<TExpr> = Vec::new();
        if let Some(bsid) = base {
            let super_call = ctor_ast.body.first().and_then(|s| match &s.kind {
                ast::StmtKind::Expr(ast::Expr { kind: ast::ExprKind::Call { callee, args, .. }, .. }) => match &callee.kind {
                    ast::ExprKind::Ident(n) if n == "super" => Some(args),
                    _ => None,
                },
                _ => None,
            });
            match super_call {
                Some(args) => {
                    rest_body = &ctor_ast.body[1..];
                    match self.classes[&bsid].ctor {
                        Some(base_ctor) => {
                            let ft = self.func_type(base_ctor, ctor_ast.span);
                            let targs = self.call_args(&ft, args, ctor_ast.span);
                            let base_val = TExpr::new(TExprKind::Call(base_ctor, targs), Type::Struct(bsid), ctor_ast.span);
                            let base_tmp = self.temp(Type::Struct(bsid));
                            prologue.push(TStmt::Let(base_tmp, Some(base_val)));
                            let read = TExpr::new(TExprKind::Var(base_tmp), Type::Struct(bsid), ctor_ast.span);
                            for i in 0..base_field_count {
                                base_field_reads.push(TExpr::new(
                                    TExprKind::Field(Box::new(read.clone()), bsid, i as u32),
                                    fields[i].ty.clone(),
                                    ctor_ast.span,
                                ));
                            }
                        }
                        None => {
                            // Already reported when the base class was declared.
                        }
                    }
                }
                None => {
                    self.err_help(
                        code::SUPER,
                        ctor_ast.span,
                        "a subclass constructor must call `super(...)` as its first statement",
                        "add `super(...)` before anything else, including any use of `this`",
                    );
                }
            }
        }

        // Each own field's value: its class-level initializer (evaluated
        // before `this` exists, so it cannot read `this`), or a
        // type-appropriate zero value for a field the constructor body
        // assigns right away (the `constructor(x: number) { this.x = x; }`
        // pattern).
        let mut field_vals = base_field_reads;
        for (f, init) in fields[base_field_count..].iter().zip(&inits[base_field_count..]) {
            let v = match init {
                Some(e) => {
                    let te = self.expr(e, Some(&f.ty));
                    self.coerce(te, &f.ty)
                }
                None => match self.zero_value(&f.ty, ctor_ast.span) {
                    Some(v) => v,
                    None => {
                        let msg = format!("field `{}` needs an initializer", f.name);
                        self.err_help(code::MISSING_FIELD, ctor_ast.span, msg, "give it a default value, or assign it in the constructor right away");
                        TExpr::new(TExprKind::Null, Type::Error, ctor_ast.span)
                    }
                },
            };
            field_vals.push(v);
        }
        let this_val = TExpr::new(TExprKind::StructLit(sid, field_vals), Type::Struct(sid), ctor_ast.span);
        let this_var = self.new_var("this", Type::Struct(sid), true);
        self.define("this", ctor_ast.span, Binding::Var(this_var));
        prologue.push(TStmt::Let(this_var, Some(this_val)));

        let mut body = prologue;
        body.extend(self.block_stmts(rest_body));
        body.push(TStmt::Return(Some(TExpr::new(TExprKind::Var(this_var), Type::Struct(sid), ctor_ast.span))));

        self.fx.scopes.pop();
        self.fx = saved_fx;
        self.prog.funcs[fid as usize].body = body;
        self.classes.get_mut(&sid).unwrap().ctor = Some(fid);
    }

    /// A type-appropriate default value for a class field without an
    /// initializer. `None` for a type with no safe zero value (structs,
    /// functions, maps, sets, unions): those need an explicit initializer.
    fn zero_value(&mut self, ty: &Type, span: Span) -> Option<TExpr> {
        match ty {
            Type::Number | Type::Int => Some(TExpr::new(TExprKind::Num(0.0), ty.clone(), span)),
            Type::Bool => Some(TExpr::new(TExprKind::Bool(false), ty.clone(), span)),
            Type::String => Some(TExpr::new(TExprKind::Str(String::new()), ty.clone(), span)),
            Type::Enum(_) => Some(TExpr::new(TExprKind::Num(0.0), ty.clone(), span)),
            Type::Array(_) => Some(TExpr::new(TExprKind::ArrayLit(Vec::new()), ty.clone(), span)),
            Type::Nullable(_) | Type::Element => {
                let n = TExpr::new(TExprKind::Null, Type::Null, span);
                Some(self.coerce(n, ty))
            }
            Type::Error => Some(TExpr::new(TExprKind::Null, Type::Error, span)),
            _ => None,
        }
    }

    /// Registers a generic top-level function as a template: it is checked
    /// (once per distinct type arguments) only when a call site
    /// instantiates it (HANDOFF.md item 3).
    fn declare_generic(&mut self, f: &ast::FuncDecl, m: usize) {
        if f.is_default {
            self.err(code::GENERIC_USER, f.span, "a generic function cannot be the default export");
        }
        let idx = self.generics.len();
        self.generics.push(GenericTemplate { decl: f.clone(), module: m });
        if let Some((n, span)) = &f.name {
            self.define(n, *span, Binding::Generic(idx));
            if f.exported {
                self.exports[m].insert(n.clone(), Binding::Generic(idx));
            }
        }
    }

    /// Checks (or reuses an earlier, identical) instantiation of a generic
    /// function template for one call site. Returns its concrete `FuncId`
    /// and `FuncType`, or `None` on an error already reported.
    pub(crate) fn instantiate_generic(
        &mut self,
        gid: usize,
        type_args: &[ast::TypeAnn],
        args: &[ast::Expr],
        span: Span,
    ) -> Option<(FuncId, Rc<FuncType>)> {
        let decl = self.generics[gid].decl.clone();
        let module = self.generics[gid].module;
        let names = decl.type_params.clone();
        // 1. The concrete type for each type parameter: explicit `f<T>(...)`
        // arguments, or inferred from the call's argument types.
        let mut bound: Vec<Type> = Vec::new();
        if !type_args.is_empty() {
            if type_args.len() != names.len() {
                self.err(code::ARG_COUNT, span, format!("`{}` takes {} type argument(s)", display_name(&decl), names.len()));
                return None;
            }
            for t in type_args {
                bound.push(self.resolve_type(t));
            }
        } else {
            let mut by_name: HashMap<&str, Type> = HashMap::new();
            for (p, a) in decl.params.iter().zip(args) {
                let Some(ann) = &p.ty else { continue };
                let te = self.expr(a, None);
                infer_type_param(ann, &te.ty, &names, &mut by_name);
            }
            for n in &names {
                match by_name.get(n.as_str()) {
                    Some(t) => bound.push(t.clone()),
                    None => {
                        let msg = format!("cannot infer type parameter `{n}` of `{}`; give it explicitly", display_name(&decl));
                        self.err(code::CANNOT_INFER, span, msg);
                        return None;
                    }
                }
            }
        }
        if bound.iter().any(Type::is_error) {
            return None;
        }
        // 2. Reuse an earlier instantiation with the same type arguments.
        if let Some((_, _, fid)) = self.instantiations.iter().find(|(g, b, _)| *g == gid && *b == bound) {
            let fid = *fid;
            return Some((fid, self.func_type(fid, span)));
        }
        // 3. Check a fresh copy of the template with `names[i] := bound[i]`.
        let saved_bindings = std::mem::take(&mut self.generic_bindings);
        for (n, t) in names.iter().zip(&bound) {
            self.generic_bindings.insert(n.clone(), t.clone());
        }
        let saved_module = std::mem::replace(&mut self.module, module);
        let base = decl.name.as_ref().map(|(n, _)| n.as_str()).unwrap_or("f");
        let mangled = format!("{base}${}", self.instantiations.iter().filter(|(g, ..)| *g == gid).count());
        let mut mono = decl.clone();
        mono.name = Some((mangled.clone(), decl.span));
        mono.type_params.clear();
        self.declare_top_func(&mono, module);
        let Some(Binding::Func(fid)) = self.lookup(&mangled) else { unreachable!() };
        self.check_pending(fid);
        self.module = saved_module;
        self.generic_bindings = saved_bindings;
        self.instantiations.push((gid, bound, fid));
        Some((fid, self.func_type(fid, span)))
    }

    /// Checks a top-level function body now (for its inferred return type).
    fn check_pending(&mut self, fid: FuncId) {
        let Some(p) = self.pending.remove(&fid) else { return };
        self.in_progress.insert(fid);
        let saved_module = std::mem::replace(&mut self.module, p.module);
        let is_async = p.decl.is_async;
        let declared = (p.decl.ret.is_some()).then(|| self.prog.funcs[fid as usize].ret.clone());
        let declared = if is_async { declared.map(|d| self.promise_inner_or_err(&d, p.decl.span)) } else { declared };
        // A component body runs one time, so a signal read there is not
        // reactive (PL2020). Components have capitalized names, as JSX needs.
        // Other functions can run inside JSX slots, `computed` or handlers,
        // where reads are fine, so they are not checked.
        let is_component = p.decl.name.as_ref().is_some_and(|(n, _)| n.starts_with(|c: char| c.is_ascii_uppercase()));
        let saved_fx = std::mem::replace(
            &mut self.fx,
            FnCx {
                func: fid,
                scopes: vec![Scope::default()],
                loops: Vec::new(),
                ret: declared.clone(),
                inferred: None,
                reactive: if is_component { ReactiveCtx::Plain } else { ReactiveCtx::Callback },
                is_async,
            },
        );
        let params = self.prog.funcs[fid as usize].params.clone();
        let mut prologue = Vec::new();
        for (p_ast, v) in p.decl.params.iter().zip(&params) {
            self.bind_param(p_ast, *v, &mut prologue);
        }
        let mut body = prologue;
        body.extend(self.func_body(&p.decl.body));
        let mut ret = self.finish_ret(declared, p.decl.span, &body);
        if is_async {
            body = self.async_body(fid, body, &ret, false, p.decl.span);
            ret = self.promise_type(&ret);
        }
        self.prog.funcs[fid as usize].ret = ret;
        self.prog.funcs[fid as usize].body = body;
        self.fx = saved_fx;
        self.module = saved_module;
        self.in_progress.remove(&fid);
    }

    /// The type of a top-level function. Checks its body when the return
    /// type is not declared.
    fn func_type(&mut self, fid: FuncId, span: Span) -> Rc<FuncType> {
        if self.pending.contains_key(&fid) && self.pending[&fid].decl.ret.is_none() {
            self.check_pending(fid);
        } else if self.in_progress.contains(&fid) && self.prog.funcs[fid as usize].ret == Type::Error {
            self.err_help(code::CANNOT_INFER, span, "a recursive function needs a return type", "add `: type` after the parameters");
        }
        let f = &self.prog.funcs[fid as usize];
        let params: Vec<Type> = f.params.iter().map(|v| self.prog.vars[*v as usize].ty.clone()).collect();
        let required = params.iter().rposition(|t| !matches!(t, Type::Nullable(_) | Type::Null)).map(|i| i + 1).unwrap_or(0);
        Rc::new(FuncType { params, required, ret: f.ret.clone() })
    }

    fn bind_param(&mut self, p: &ast::Param, v: VarId, prologue: &mut Vec<TStmt>) {
        match &p.pattern {
            ast::Pattern::Ident(name, span) => self.define(name, *span, Binding::Var(v)),
            pat => {
                let src = TExpr::new(TExprKind::Var(v), self.prog.vars[v as usize].ty.clone(), p.span);
                let stmts = self.destructure(pat, src, false);
                prologue.extend(stmts);
            }
        }
    }

    fn func_body(&mut self, body: &ast::Body) -> Vec<TStmt> {
        match body {
            ast::Body::Block(stmts) => self.block_stmts(stmts),
            ast::Body::Expr(e) => {
                let expected = self.fx.ret.clone();
                let mut te = self.expr(e, expected.as_ref());
                if expected == Some(Type::Void) {
                    if te.ty != Type::Void {
                        let span = te.span;
                        te = TExpr::new(TExprKind::Coerce(Coercion::Discard, Box::new(te)), Type::Void, span);
                    }
                    return vec![TStmt::Expr(te)];
                }
                if let Some(r) = &expected {
                    te = self.coerce(te, r);
                } else {
                    self.fx.inferred = Some(te.ty.clone());
                }
                if te.ty == Type::Void { vec![TStmt::Expr(te)] } else { vec![TStmt::Return(Some(te))] }
            }
        }
    }

    /// The final return type, and a check that a non-void function returns.
    fn finish_ret(&mut self, declared: Option<Type>, span: Span, body: &[TStmt]) -> Type {
        let ret = declared.or(self.fx.inferred.take()).unwrap_or(Type::Void);
        if ret != Type::Void && !ret.is_error() && !always_exits(body) {
            self.err_help(
                code::MISSING_RETURN,
                span,
                format!("this function must return a value of type `{}` on every path", self.show(&ret)),
                "add a `return` at the end",
            );
        }
        ret
    }

    /// Checks an arrow function or function expression. `expected` gives
    /// parameter types for untyped parameters (contextual typing).
    pub(crate) fn closure(&mut self, f: &ast::FuncDecl, expected: Option<&FuncType>, name: &str) -> TExpr {
        let fid = self.prog.new_func(FuncDef {
            name: name.to_owned(),
            kind: FuncKind::Closure,
            params: Vec::new(),
            ret: Type::Error,
            body: Vec::new(),
            span: f.span,
        });
        let mut param_vars = Vec::new();
        let mut param_types = Vec::new();
        let mut required = 0;
        for (i, p) in f.params.iter().enumerate() {
            if p.default.is_some() {
                self.err_help(
                    code::UNSUPPORTED,
                    p.span,
                    "default values on parameters of function values are not supported yet",
                    "make the parameter optional (`x?: T`) and use `??`",
                );
            }
            let mut ty = match (&p.ty, expected.and_then(|e| e.params.get(i))) {
                (Some(t), _) => self.resolve_type(t),
                (None, Some(t)) => t.clone(),
                (None, None) => {
                    self.err_help(code::CANNOT_INFER, p.span, "this parameter needs a type", "add `: type`");
                    Type::Error
                }
            };
            if p.optional {
                ty = self.nullable(ty, p.span);
            } else {
                required = i + 1;
            }
            let pname = match &p.pattern {
                ast::Pattern::Ident(n, _) => n.clone(),
                _ => format!("$p{i}"),
            };
            param_vars.push(self.prog.new_var(VarInfo {
                name: pname,
                ty: ty.clone(),
                owner: fid,
                mutable: true,
                module: None,
                in_loop: None,
                captured: false,
            }));
            param_types.push(ty);
        }
        let declared = match &f.ret {
            Some(r) => Some(self.resolve_type(r)),
            None => expected.map(|e| e.ret.clone()).filter(|t| *t == Type::Void || !t.is_error()),
        };
        // An `async` closure where a `void` function is expected (an event
        // handler, `forEach`, …) returns nothing: its promise is detached.
        let detached = f.is_async && f.ret.is_none() && declared == Some(Type::Void);
        let declared = match declared {
            Some(d) if f.is_async && !detached => Some(self.promise_inner_or_err(&d, f.span)),
            d => d,
        };
        let reactive = if std::mem::take(&mut self.pending_reactive) { ReactiveCtx::Reactive } else { ReactiveCtx::Callback };
        // Set before checking the body (not just after, at the bottom):
        // member-path narrowing (`check/stmt.rs`'s `narrow_path`) looks up
        // `self.prog.funcs[owner].params` to tell a parameter from a
        // captured `let`, and it runs while the body below is checked.
        self.prog.funcs[fid as usize].params = param_vars.clone();
        let mut saved =
            std::mem::replace(&mut self.fx, FnCx { func: fid, scopes: Vec::new(), loops: Vec::new(), ret: declared.clone(), inferred: None, reactive, is_async: f.is_async });
        // A closure sees the scopes of the enclosing function: move them in
        // for the body, then give them back.
        self.fx.scopes = std::mem::take(&mut saved.scopes);
        self.fx.scopes.push(Scope::default());
        let mut prologue = Vec::new();
        for (p, v) in f.params.iter().zip(&param_vars) {
            self.bind_param(p, *v, &mut prologue);
        }
        let mut body = prologue;
        body.extend(self.func_body(&f.body));
        let mut ret = self.finish_ret(declared, f.span, &body);
        self.fx.scopes.pop();
        saved.scopes = std::mem::take(&mut self.fx.scopes);
        self.fx = saved;
        if f.is_async {
            body = self.async_body(fid, body, &ret, detached, f.span);
            ret = if detached { Type::Void } else { self.promise_type(&ret) };
        }
        let def = &mut self.prog.funcs[fid as usize];
        def.params = param_vars;
        def.ret = ret.clone();
        def.body = body;
        let ft = FuncType { params: param_types, required, ret };
        TExpr::new(TExprKind::Closure(fid), Type::Func(Rc::new(ft)), f.span)
    }
}

/// True if a statement list always ends in `return` or `throw`.
pub(crate) fn always_exits(stmts: &[TStmt]) -> bool {
    stmts.last().is_some_and(|s| match s {
        TStmt::Return(_) | TStmt::Throw(_) | TStmt::Trap(_) => true,
        TStmt::Try { body, catch, finally } => {
            (always_exits(body) && catch.as_ref().is_none_or(|(_, c)| always_exits(c))) || finally.as_ref().is_some_and(|f| always_exits(f))
        }
        TStmt::If(_, a, b) => always_exits(a) && always_exits(b),
        TStmt::Block(b) => always_exits(b),
        TStmt::Switch { disc, cases, .. } => {
            (cases.iter().any(|(t, _)| t.is_none()) || covers_all(disc, cases))
                && cases.iter().all(|(_, b)| b.is_empty() || always_exits(b))
                && cases.last().is_some_and(|(_, b)| always_exits(b))
        }
        TStmt::Loop { cond: None, body, .. } => !contains_break(body),
        _ => false,
    })
}

/// True if the cases name every member of a string literal union.
fn covers_all(disc: &TExpr, cases: &[(Option<TExpr>, Vec<TStmt>)]) -> bool {
    let Type::StrLits(lits) = &disc.ty else { return false };
    let labels: Vec<&str> = cases
        .iter()
        .filter_map(|(t, _)| {
            let mut e = t.as_ref()?;
            while let TExprKind::Coerce(Coercion::Retag, inner) = &e.kind {
                e = inner;
            }
            match &e.kind {
                TExprKind::Str(s) => Some(s.as_str()),
                _ => None,
            }
        })
        .collect();
    lits.iter().all(|l| labels.contains(&l.as_str()))
}

fn contains_break(stmts: &[TStmt]) -> bool {
    stmts.iter().any(|s| match s {
        TStmt::Break => true,
        TStmt::If(_, a, b) => contains_break(a) || contains_break(b),
        TStmt::Block(b) => contains_break(b),
        TStmt::Try { body, catch, finally } => {
            contains_break(body) || catch.as_ref().is_some_and(|(_, c)| contains_break(c)) || finally.as_ref().is_some_and(|f| contains_break(f))
        }
        _ => false,
    })
}

/// The synthetic `this: ClassName` first parameter of a lowered method.
fn this_param(class_name: &str, span: Span) -> ast::Param {
    ast::Param {
        pattern: ast::Pattern::Ident("this".to_string(), span),
        ty: Some(TypeAnn::Named { name: class_name.to_string(), args: Vec::new(), span }),
        default: None,
        optional: false,
        span,
    }
}

/// True if `s` (or something nested in it, not counting a nested function
/// or arrow body) is a `return` statement.
fn stmt_has_return(s: &ast::Stmt) -> bool {
    use ast::StmtKind as K;
    match &s.kind {
        K::Return(_) => true,
        K::Block(b) => b.iter().any(stmt_has_return),
        K::If(_, a, b) => stmt_has_return(a) || b.as_deref().is_some_and(stmt_has_return),
        K::While(_, b) | K::DoWhile(b, _) => stmt_has_return(b),
        K::For { body, .. } | K::ForOf { body, .. } => stmt_has_return(body),
        K::Switch(_, cases) => cases.iter().any(|(_, b)| b.iter().any(stmt_has_return)),
        _ => false,
    }
}

fn display_name(decl: &ast::FuncDecl) -> &str {
    decl.name.as_ref().map(|(n, _)| n.as_str()).unwrap_or("<anonymous>")
}

/// Unifies a parameter's type annotation (as written, with type-parameter
/// names still bare identifiers) against the checked type of the argument,
/// to infer `name → Type` for any type parameter it mentions. Only the
/// shapes generic helpers actually need: `T`, `T[]`, `T | null`.
fn infer_type_param<'a>(ann: &'a TypeAnn, arg: &Type, names: &[String], out: &mut HashMap<&'a str, Type>) {
    match ann {
        TypeAnn::Named { name, args, .. } if args.is_empty() && names.iter().any(|n| n == name) => {
            out.entry(name.as_str()).or_insert_with(|| arg.clone());
        }
        TypeAnn::Array(inner, _) => {
            if let Type::Array(elem) = arg {
                infer_type_param(inner, elem, names, out);
            }
        }
        TypeAnn::Union(parts, _) => {
            // `T | null`: unify `T` against the non-null part of the arg.
            let inner_arg = match arg {
                Type::Nullable(t) => (**t).clone(),
                t => t.clone(),
            };
            for p in parts {
                if !matches!(p, TypeAnn::Null(_)) {
                    infer_type_param(p, &inner_arg, names, out);
                }
            }
        }
        _ => {}
    }
}
