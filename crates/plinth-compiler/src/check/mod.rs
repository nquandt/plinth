//! The Plinth TS type checker (SPEC.md §5.1 step 3).
//!
//! It checks the modules in dependency order and produces the typed IR
//! (`tir::Program`). Each rejected feature has a stable code (`diag::code`).

mod expr;
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
}

impl StdModule {
    pub fn from_specifier(s: &str) -> Option<Self> {
        match s {
            "plinth:ui" => Some(StdModule::Ui),
            "plinth:core" => Some(StdModule::Core),
            "plinth:time" => Some(StdModule::Time),
            "plinth:store" => Some(StdModule::Store),
            "plinth:clipboard" => Some(StdModule::Clipboard),
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
    App,
    Navigate,
    ParseNumber,
    ToString,
    TimeNow,
    TimeMonotonicNow,
    SetTimeout,
    SetInterval,
    ClearTimer,
    ClipboardWriteText,
    ClipboardReadText,
    ClipboardLastError,
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
}

#[derive(Debug, Clone)]
pub enum Binding {
    Var(VarId),
    Func(FuncId),
    Type(Type),
    /// A type alias that is not resolved yet: `(module, alias index)`.
    Alias(usize, usize),
    Enum(types::EnumId),
    Control(ControlKind),
    Std(StdFn),
    StdObj(StdObj),
}

#[derive(Default)]
struct Scope {
    names: HashMap<String, Binding>,
    /// Narrowed types of variables (`if (x !== null)`).
    narrow: HashMap<VarId, Type>,
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
}

/// Checks all modules. `modules` must be in dependency order (dependencies
/// first); `main` is the index of `app/main.tsx`. `capabilities` are the
/// capability names declared in the project's `plinth.toml`.
pub fn check(modules: &[ModuleSrc], main: usize, diags: &mut Vec<Diagnostic>, capabilities: &[String]) -> Program {
    let mut c = Checker {
        prog: Program::default(),
        diags,
        exports: vec![HashMap::new(); modules.len()],
        defaults: vec![None; modules.len()],
        module_scopes: vec![HashMap::new(); modules.len()],
        module: 0,
        fx: FnCx { func: 0, scopes: Vec::new(), loops: Vec::new(), ret: None, inferred: None },
        pending: HashMap::new(),
        in_progress: HashSet::new(),
        aliases: vec![Vec::new(); modules.len()],
        resolving_alias: HashSet::new(),
        anon_structs: HashMap::new(),
        navigations: Vec::new(),
        app_seen: false,
        capabilities: capabilities.iter().cloned().collect(),
    };
    c.prog.module_count = modules.len() as u32;
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
        self.fx = FnCx { func: init, scopes: Vec::new(), loops: Vec::new(), ret: Some(Type::Void), inferred: None };

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

        // 2. Types: enums and interfaces first (by name), then fields.
        let mut interfaces = Vec::new();
        for item in &src.ast.items {
            match item {
                Item::Enum(e) => self.declare_enum(e),
                Item::Interface(i) => {
                    let id = self.prog.structs.len() as types::StructId;
                    self.prog.structs.push(StructDef { name: i.name.clone(), fields: Vec::new() });
                    self.define(&i.name, i.span, Binding::Type(Type::Struct(id)));
                    if i.exported {
                        self.exports[m].insert(i.name.clone(), Binding::Type(Type::Struct(id)));
                    }
                    interfaces.push((id, i));
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

        // 3. Top-level functions are hoisted.
        for item in &src.ast.items {
            if let Item::Stmt(ast::Stmt { kind: ast::StmtKind::Func(f), .. }) = item {
                self.declare_top_func(f, m);
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
        self.module_scopes[self.module].get(name).cloned()
    }

    fn narrowed(&self, v: VarId) -> Option<Type> {
        for s in self.fx.scopes.iter().rev() {
            if let Some(t) = s.narrow.get(&v) {
                return Some(t.clone());
            }
        }
        None
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
    fn nullable(&mut self, ty: Type, span: Span) -> Type {
        match ty {
            Type::Bool | Type::Enum(_) | Type::Signal(_) | Type::Computed(_) => {
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
                        } else {
                            self.err_help(
                                code::ADVANCED_TYPE,
                                *span,
                                "general union types come in v1",
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
            TypeAnn::Named { name, args, span } => self.named_type(name, args, *span),
        }
    }

    fn named_type(&mut self, name: &str, args: &[TypeAnn], span: Span) -> Type {
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
            "int" => {
                self.err_help(code::UNSUPPORTED, span, "`int` is not supported yet", "use `number`");
                return Type::Error;
            }
            "any" | "unknown" | "object" | "Object" => {
                self.err(code::ANY, span, format!("`{name}` is not allowed"));
                return Type::Error;
            }
            "Map" | "Set" | "Promise" | "Record" | "Partial" | "Readonly" => {
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
            Some(Binding::Alias(m, idx)) => self.resolve_alias(m, idx, span),
            Some(Binding::Enum(e)) => Type::Enum(e),
            _ => {
                self.err(code::UNKNOWN_TYPE, span, format!("unknown type `{name}`"));
                Type::Error
            }
        }
    }

    fn resolve_alias(&mut self, m: usize, idx: usize, span: Span) -> Type {
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
            (T::StrLits(_), T::String) => Some(Some(Coercion::Retag)),
            (T::StrLits(a), T::StrLits(b)) if a.iter().all(|x| b.contains(x)) => Some(Some(Coercion::Retag)),
            (T::Null, T::Nullable(_)) | (T::Null, T::Element) => Some(Some(Coercion::Retag)),
            (T::Number, T::Nullable(inner)) if **inner == T::Number => Some(Some(Coercion::BoxNum)),
            (t, T::Nullable(inner)) if t.repr() == types::Repr::Ref => {
                self.conversion(t, inner).filter(|c| c.is_none_or(|c| c == Coercion::Retag)).map(|_| Some(Coercion::Retag))
            }
            (T::Nullable(a), T::Nullable(b)) => {
                self.conversion(a, b).filter(|c| c.is_none_or(|c| c == Coercion::Retag)).map(|_| Some(Coercion::Retag))
            }
            (T::Struct(a), T::Struct(b)) => self.same_layout(*a, *b).then_some(Some(Coercion::Retag)),
            (T::Array(a), T::Array(b)) => {
                (a.repr() == b.repr() && self.conversion(a, b).is_some_and(|c| c.is_none_or(|c| c == Coercion::Retag)))
                    .then_some(Some(Coercion::Retag))
            }
            (T::Func(f), T::Func(g)) => self.func_compatible(f, g).then_some(Some(Coercion::Retag)),
            _ => None,
        }
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
            Type::Enum(_) => {
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

    /// Checks a top-level function body now (for its inferred return type).
    fn check_pending(&mut self, fid: FuncId) {
        let Some(p) = self.pending.remove(&fid) else { return };
        self.in_progress.insert(fid);
        let saved_module = std::mem::replace(&mut self.module, p.module);
        let declared = (p.decl.ret.is_some()).then(|| self.prog.funcs[fid as usize].ret.clone());
        let saved_fx = std::mem::replace(
            &mut self.fx,
            FnCx { func: fid, scopes: vec![Scope::default()], loops: Vec::new(), ret: declared.clone(), inferred: None },
        );
        let params = self.prog.funcs[fid as usize].params.clone();
        let mut prologue = Vec::new();
        for (p_ast, v) in p.decl.params.iter().zip(&params) {
            self.bind_param(p_ast, *v, &mut prologue);
        }
        let mut body = prologue;
        body.extend(self.func_body(&p.decl.body));
        let ret = self.finish_ret(declared, p.decl.span, &body);
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
        let mut saved = std::mem::replace(
            &mut self.fx,
            FnCx { func: fid, scopes: Vec::new(), loops: Vec::new(), ret: declared.clone(), inferred: None },
        );
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
        let ret = self.finish_ret(declared, f.span, &body);
        self.fx.scopes.pop();
        saved.scopes = std::mem::take(&mut self.fx.scopes);
        self.fx = saved;
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
        TStmt::Return(_) | TStmt::Throw(_) => true,
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
        _ => false,
    })
}
