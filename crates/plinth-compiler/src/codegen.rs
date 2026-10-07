//! Wasm code generation (SPEC.md §5.1 step 7) for the lowered typed IR.
//!
//! # Conventions
//!
//! - Every function takes an environment as its first parameter (`i32`).
//!   Top-level functions ignore it; direct calls pass 0.
//! - A closure value is a heap object `{ fn: table index, env }`.
//! - A variable that a nested function uses (a captured variable) lives in a
//!   heap **frame**: one frame per function call, and one per loop iteration
//!   for variables declared in a loop body. A frame starts with a pointer to
//!   the enclosing frame. Other variables are Wasm locals.
//! - Module variables live in one environment object per module, which a
//!   rooted global holds.
//! - Thunks adapt closures to the runtime's `(env) -> ()` calls.

use crate::link::{AppCode, Layout};
use crate::rt_abi::{ARR_F64, ARR_I32, ARR_REF, FIRST_USER_TYPE, HEADER, STR_BYTES};
use crate::tir::*;
use crate::types::{Repr, StructId, Type};
use std::collections::{HashMap, HashSet, VecDeque};
use wasm_encoder::{BlockType, ConstExpr, Function, Instruction as I, MemArg, ValType};

type FrameKey = (FuncId, Option<LoopId>);

struct FrameDesc {
    parent: Option<usize>,
    type_id: u32,
    slots: HashMap<VarId, (u32, Repr)>,
}

/// A compiler-defined heap type for the runtime's type table.
struct UserType {
    size: u32,
    ref_offsets: Vec<u32>,
}

struct StructLayout {
    /// The GC type id, given on first use (`sid_type`), so a struct that
    /// the program never makes or tests costs no type-table entry.
    type_id: Option<u32>,
    size: u32,
    refs: Vec<u32>,
    offsets: Vec<(u32, Repr)>,
}

pub struct Codegen<'p> {
    prog: &'p Program,
    layout: &'p Layout,
    types: Vec<(Vec<ValType>, Vec<ValType>)>,
    type_index: HashMap<(Vec<ValType>, Vec<ValType>), u32>,
    /// FuncId → absolute function index.
    func_index: HashMap<FuncId, u32>,
    /// Function index → body, in index order.
    bodies: Vec<Option<(u32, Function)>>,
    queue: VecDeque<FuncId>,
    /// The frame that a closure's env parameter points to.
    env_desc: HashMap<FuncId, Option<usize>>,
    table: Vec<u32>,
    table_index: HashMap<u32, u32>,
    thunks: HashMap<ThunkSig, u32>,
    user_types: Vec<UserType>,
    structs: Vec<StructLayout>,
    frames: Vec<FrameDesc>,
    frame_by_key: HashMap<FrameKey, usize>,
    module_env: HashMap<u32, (u32, usize)>, // module → (global, frame desc)
    globals: Vec<(ValType, ConstExpr)>,
    literals: Vec<String>,
    literal_index: HashMap<String, u32>,
    /// Captured variables (used by a function other than the owner).
    captured: HashSet<VarId>,
    /// Exceptions (SPEC.md §5.6): the functions that can return with a
    /// pending exception. Only a call of one of these needs a check after
    /// it, so an app with no `throw` pays nothing.
    may_throw: HashSet<FuncId>,
    /// True if a closure value (any function used as a value) can throw:
    /// then every closure call and every thunk checks for an exception.
    closures_throw: bool,
    /// Method names with an implementation that can throw.
    throwing_methods: HashSet<String>,
    /// The `(flag, value)` globals of the pending exception, made on first
    /// use. `flag` is 1 while an exception is pending; `value` is the
    /// thrown `Error`.
    exc: Option<(u32, u32)>,
}

/// A `finally` block that a `return`, `break` or `continue` inside its
/// `try` must run first.
#[derive(Clone)]
struct FinallyCx {
    body: Vec<TStmt>,
    /// `FnGen::labels.len()` at the `try`: a jump to a label below this
    /// index leaves the `try`.
    labels: usize,
    /// `FnGen::handlers.len()` outside the `try`.
    handlers: usize,
}

fn mem(offset: u32, repr: Repr) -> MemArg {
    MemArg { offset: offset as u64, align: if repr == Repr::F64 { 3 } else { 2 }, memory_index: 0 }
}

fn load(offset: u32, repr: Repr) -> I<'static> {
    match repr {
        Repr::F64 => I::F64Load(mem(offset, repr)),
        _ => I::I32Load(mem(offset, repr)),
    }
}

fn store(offset: u32, repr: Repr) -> I<'static> {
    match repr {
        Repr::F64 => I::F64Store(mem(offset, repr)),
        _ => I::I32Store(mem(offset, repr)),
    }
}

fn vt(r: Repr) -> Vec<ValType> {
    r.val_type().into_iter().collect()
}

fn block_type(r: Repr) -> BlockType {
    match r.val_type() {
        Some(t) => BlockType::Result(t),
        None => BlockType::Empty,
    }
}

/// Computes field offsets: the header, then each field aligned to its size.
fn lay_out(fields: &[Repr], start: u32) -> (Vec<u32>, u32) {
    let mut off = start;
    let mut offsets = Vec::new();
    for r in fields {
        let size = if *r == Repr::F64 { 8 } else { 4 };
        off = off.div_ceil(size) * size;
        offsets.push(off);
        off += size;
    }
    (offsets, off.div_ceil(8) * 8)
}

/// The instructions that push the table index `slot`. In an app module the
/// index is relative to the imported `table_base` global (SPEC.md §10.4);
/// the host's link step turns `global.get` into a constant.
fn table_ref(layout: &Layout, slot: u32) -> Vec<I<'static>> {
    match layout.table_base_global {
        Some(g) if slot == 0 => vec![I::GlobalGet(g)],
        Some(g) => vec![I::GlobalGet(g), I::I32Const(slot as i32), I::I32Add],
        None => vec![I::I32Const(slot as i32)],
    }
}

pub fn generate(prog: &Program, layout: &Layout, main: FuncId) -> AppCode {
    let mut g = Codegen {
        prog,
        layout,
        types: Vec::new(),
        type_index: HashMap::new(),
        func_index: HashMap::new(),
        bodies: Vec::new(),
        queue: VecDeque::new(),
        env_desc: HashMap::new(),
        table: Vec::new(),
        table_index: HashMap::new(),
        thunks: HashMap::new(),
        user_types: Vec::new(),
        structs: Vec::new(),
        frames: Vec::new(),
        frame_by_key: HashMap::new(),
        module_env: HashMap::new(),
        globals: Vec::new(),
        literals: Vec::new(),
        literal_index: HashMap::new(),
        captured: HashSet::new(),
        may_throw: HashSet::new(),
        closures_throw: false,
        throwing_methods: HashSet::new(),
        exc: None,
    };
    g.find_captures();
    g.find_throwers();
    g.lay_out_structs();
    g.lay_out_module_envs();

    // The entry thunk and the start function come first, so their indices
    // are known; their bodies are filled in at the end.
    let entry_idx = g.reserve_func();
    let start_idx = g.reserve_func();
    let main_idx = g.func_ref(main);
    while let Some(f) = g.queue.pop_front() {
        g.compile(f);
    }
    // Thunks and frames can add more functions only through the queue, which
    // is empty now.
    let entry_body = g.entry(main_idx, main);
    let thunk_ty = g.type_of(vec![ValType::I32], vec![]);
    g.set_body(entry_idx, thunk_ty, entry_body);
    let entry_table = g.table_slot(entry_idx);
    let mut start = Function::new(Vec::new());
    if let Some(s) = layout.rt_start {
        start.instruction(&I::Call(s));
    }
    for i in table_ref(layout, entry_table) {
        start.instruction(&i);
    }
    start.instruction(&I::Call(layout.rt("set_main")));
    start.instruction(&I::End);
    let void_ty = g.type_of(vec![], vec![]);
    g.set_body(start_idx, void_ty, start);

    let mut data = Vec::new();
    data.push(g.literal_bytes());
    data.push(g.type_table_bytes());
    AppCode {
        types: g.types,
        funcs: g.bodies.into_iter().map(|b| b.expect("every reserved function has a body")).collect(),
        globals: g.globals,
        table: g.table,
        data,
        start: Some(start_idx),
    }
}

impl<'p> Codegen<'p> {
    // -- Index spaces -------------------------------------------------------

    fn type_of(&mut self, params: Vec<ValType>, results: Vec<ValType>) -> u32 {
        let key = (params, results);
        if let Some(i) = self.type_index.get(&key) {
            return *i;
        }
        let idx = self.layout.type_count + self.types.len() as u32;
        self.types.push(key.clone());
        self.type_index.insert(key, idx);
        idx
    }

    fn reserve_func(&mut self) -> u32 {
        self.bodies.push(None);
        self.layout.func_count + self.bodies.len() as u32 - 1
    }

    fn set_body(&mut self, idx: u32, ty: u32, f: Function) {
        self.bodies[(idx - self.layout.func_count) as usize] = Some((ty, f));
    }

    /// The function index of a TIR function; queues it for compilation.
    fn func_ref(&mut self, fid: FuncId) -> u32 {
        if let Some(i) = self.func_index.get(&fid) {
            return *i;
        }
        let idx = self.reserve_func();
        self.func_index.insert(fid, idx);
        self.queue.push_back(fid);
        idx
    }

    fn table_slot(&mut self, func: u32) -> u32 {
        if let Some(i) = self.table_index.get(&func) {
            return *i;
        }
        let i = self.layout.table_size + self.table.len() as u32;
        self.table.push(func);
        self.table_index.insert(func, i);
        i
    }

    fn global(&mut self) -> u32 {
        let idx = self.layout.global_count + self.globals.len() as u32;
        self.globals.push((ValType::I32, ConstExpr::i32_const(0)));
        idx
    }

    fn literal(&mut self, s: &str) -> u32 {
        if let Some(g) = self.literal_index.get(s) {
            return *g;
        }
        let g = self.global();
        self.literals.push(s.to_owned());
        self.literal_index.insert(s.to_owned(), g);
        g
    }

    fn user_type(&mut self, size: u32, ref_offsets: Vec<u32>) -> u32 {
        self.user_types.push(UserType { size, ref_offsets });
        FIRST_USER_TYPE + self.user_types.len() as u32 - 1
    }

    /// The GC type id of struct `sid`.
    fn sid_type(&mut self, sid: StructId) -> u32 {
        if let Some(t) = self.structs[sid as usize].type_id {
            return t;
        }
        let (size, refs) = (self.structs[sid as usize].size, self.structs[sid as usize].refs.clone());
        let t = self.user_type(size, refs);
        self.structs[sid as usize].type_id = Some(t);
        t
    }

    /// The GC type id a union member has at runtime, to narrow via the
    /// header's `type_id` (`UnionIs`).
    fn type_id_of(&mut self, member: &Type) -> u32 {
        match member {
            Type::Struct(sid) => self.sid_type(*sid),
            Type::String | Type::StrLits(_) => crate::rt_abi::T_STRING,
            other => panic!("a union member of type `{other:?}` has no runtime tag yet"),
        }
    }

    // -- Classes (SPEC.md §4.2 v1) -------------------------------------------

    /// `sid` and every (transitive) subclass of it, including itself.
    fn class_descendants(&self, sid: StructId) -> Vec<StructId> {
        let mut out = vec![sid];
        loop {
            let mut added = false;
            for (&s, info) in &self.prog.classes {
                if out.contains(&s) {
                    continue;
                }
                if info.base.is_some_and(|b| out.contains(&b)) {
                    out.push(s);
                    added = true;
                }
            }
            if !added {
                break;
            }
        }
        out
    }

    /// The method `name` as class `sid` resolves it: its own, or inherited
    /// from the nearest base that defines it.
    fn effective_method(&self, sid: StructId, name: &str) -> Option<FuncId> {
        let mut cur = Some(sid);
        while let Some(s) = cur {
            let info = self.prog.classes.get(&s)?;
            if let Some(&fid) = info.methods.get(name) {
                return Some(fid);
            }
            cur = info.base;
        }
        None
    }

    /// The call targets for a method call through a value of static class
    /// `sid`: one `(0, func index)` when every reachable subclass resolves
    /// to the same implementation (plain static dispatch, no overhead for
    /// a class with no overrides), or one `(type id, func index)` per
    /// distinct implementation otherwise, for an inline runtime dispatch.
    fn dispatch_targets(&mut self, sid: StructId, name: &str) -> Vec<(u32, u32)> {
        let mut raw: Vec<(StructId, FuncId)> =
            self.class_descendants(sid).into_iter().filter_map(|d| self.effective_method(d, name).map(|f| (d, f))).collect();
        if raw.is_empty() {
            // The checker already verified `name` is reachable from `sid`.
            if let Some(fid) = self.effective_method(sid, name) {
                raw.push((sid, fid));
            }
        }
        let all_same = raw.iter().all(|(_, f)| *f == raw[0].1);
        if all_same {
            let idx = self.func_ref(raw[0].1);
            return vec![(0, idx)];
        }
        raw.into_iter()
            .map(|(d, fid)| {
                let idx = self.func_ref(fid);
                (self.sid_type(d), idx)
            })
            .collect()
    }

    fn sig_type(&mut self, ft: &crate::types::FuncType) -> u32 {
        let mut params = vec![ValType::I32];
        params.extend(ft.params.iter().filter_map(|t| t.repr().val_type()));
        let results = vt(ft.ret.repr());
        self.type_of(params, results)
    }

    // -- Layouts ------------------------------------------------------------

    fn lay_out_structs(&mut self) {
        for s in &self.prog.structs {
            let reprs: Vec<Repr> = s.fields.iter().map(|f| f.ty.repr()).collect();
            let (offsets, size) = lay_out(&reprs, HEADER);
            let refs = offsets.iter().zip(&reprs).filter(|(_, r)| **r == Repr::Ref).map(|(o, _)| *o).collect();
            self.structs.push(StructLayout { type_id: None, size, refs, offsets: offsets.into_iter().zip(reprs).collect() });
        }
    }

    fn lay_out_module_envs(&mut self) {
        let mut by_module: HashMap<u32, Vec<VarId>> = HashMap::new();
        for (v, info) in self.prog.vars.iter().enumerate() {
            if let Some(m) = info.module {
                by_module.entry(m).or_default().push(v as VarId);
            }
        }
        let mut modules: Vec<_> = by_module.into_iter().collect();
        modules.sort_by_key(|(m, _)| *m);
        for (m, vars) in modules {
            let desc = self.new_frame((u32::MAX - m, None), None, &vars, false);
            let g = self.global();
            self.module_env.insert(m, (g, desc));
        }
    }

    fn new_frame(&mut self, key: FrameKey, parent: Option<usize>, vars: &[VarId], with_parent: bool) -> usize {
        let reprs: Vec<Repr> = vars.iter().map(|v| self.prog.vars[*v as usize].ty.repr()).collect();
        let start = if with_parent { HEADER + 8 } else { HEADER };
        let (offsets, size) = lay_out(&reprs, start);
        let mut refs: Vec<u32> = offsets.iter().zip(&reprs).filter(|(_, r)| **r == Repr::Ref).map(|(o, _)| *o).collect();
        if with_parent {
            refs.push(HEADER);
        }
        let type_id = self.user_type(size, refs);
        let slots = vars.iter().zip(offsets.iter().zip(reprs)).map(|(v, (o, r))| (*v, (*o, r))).collect();
        self.frames.push(FrameDesc { parent, type_id, slots });
        let id = self.frames.len() - 1;
        self.frame_by_key.insert(key, id);
        id
    }

    // -- Capture analysis -------------------------------------------------------

    fn find_captures(&mut self) {
        for (fid, f) in self.prog.funcs.iter().enumerate() {
            let fid = fid as FuncId;
            let mut uses = Vec::new();
            for s in &f.body {
                stmt_vars(s, &mut uses);
            }
            for v in uses {
                let info = &self.prog.vars[v as usize];
                if info.owner != fid && info.module.is_none() {
                    self.captured.insert(v);
                }
            }
        }
    }

    // -- Entry ----------------------------------------------------------------

    fn entry(&mut self, main_idx: u32, main: FuncId) -> Function {
        let rt = |n: &str| self.layout.rt(n);
        let mut f = Function::new(vec![(1, ValType::I32)]);
        // 1. The type table.
        let words = self.type_table_words() as i32;
        let types_seg = self.layout.data_count + 1;
        f.instruction(&I::I32Const(words * 4));
        f.instruction(&I::Call(rt("scratch")));
        f.instruction(&I::LocalTee(1));
        f.instruction(&I::I32Const(0));
        f.instruction(&I::I32Const(words * 4));
        f.instruction(&I::MemoryInit { mem: 0, data_index: types_seg });
        f.instruction(&I::LocalGet(1));
        f.instruction(&I::I32Const(words));
        f.instruction(&I::Call(rt("types")));
        // 2. String literals: allocate, copy, and root.
        let lit_seg = self.layout.data_count;
        let mut offset = 0i32;
        for s in &self.literals {
            let g = self.literal_index[s];
            let len = s.len() as i32;
            f.instruction(&I::I32Const(len));
            f.instruction(&I::Call(rt("str_new")));
            f.instruction(&I::GlobalSet(g));
            f.instruction(&I::GlobalGet(g));
            f.instruction(&I::I32Const(STR_BYTES as i32));
            f.instruction(&I::I32Add);
            f.instruction(&I::I32Const(offset));
            f.instruction(&I::I32Const(len));
            f.instruction(&I::MemoryInit { mem: 0, data_index: lit_seg });
            f.instruction(&I::GlobalGet(g));
            f.instruction(&I::Call(rt("root")));
            offset += len;
        }
        // 3. Module environments.
        let mut envs: Vec<_> = self.module_env.values().copied().collect();
        envs.sort();
        for (g, desc) in envs {
            f.instruction(&I::I32Const(self.frames[desc].type_id as i32));
            f.instruction(&I::Call(rt("alloc")));
            f.instruction(&I::GlobalSet(g));
            f.instruction(&I::GlobalGet(g));
            f.instruction(&I::Call(rt("root")));
        }
        // 4. Module initializers and the screens.
        f.instruction(&I::I32Const(0));
        f.instruction(&I::Call(main_idx));
        if self.may_throw.contains(&main) {
            // An uncaught exception in the start-up code (a module's
            // top-level statements or a screen's component): report it,
            // then stop, because the app has no complete UI (SPEC.md §5.6).
            let mut report = self.report_uncaught();
            report.insert(report.len() - 1, I::Unreachable);
            for i in report {
                f.instruction(&i);
            }
        }
        f.instruction(&I::End);
        f
    }

    // -- Exceptions (SPEC.md §5.6) --------------------------------------------

    /// The `(flag, value)` globals of the pending exception.
    fn exc_globals(&mut self) -> (u32, u32) {
        if let Some(e) = self.exc {
            return e;
        }
        let e = (self.global(), self.global());
        self.exc = Some(e);
        e
    }

    /// Code for a boundary where app code returns to the runtime (a thunk,
    /// the entry): if an exception is pending, clear it and report it to
    /// the host as uncaught. The app keeps running.
    fn report_uncaught(&mut self) -> Vec<I<'static>> {
        let (flag, val) = self.exc_globals();
        let sid = self.prog.error_class.expect("a program that throws has the Error class");
        let (name_off, _) = self.structs[sid as usize].offsets[0];
        let (msg_off, _) = self.structs[sid as usize].offsets[1];
        vec![
            I::GlobalGet(flag),
            I::If(BlockType::Empty),
            I::I32Const(0),
            I::GlobalSet(flag),
            I::GlobalGet(val),
            I::I32Load(mem(name_off, Repr::Ref)),
            I::GlobalGet(val),
            I::I32Load(mem(msg_off, Repr::Ref)),
            I::Call(self.layout.rt("uncaught")),
            I::End,
        ]
    }

    /// The may-throw analysis: a function can throw if it has a `throw`, or
    /// calls a function, a closure or a method that can throw. Runtime
    /// functions never throw: a callback that the runtime calls reports
    /// its own uncaught exception at its thunk.
    fn find_throwers(&mut self) {
        struct Facts {
            throws: bool,
            calls: Vec<FuncId>,
            closure_calls: bool,
            methods: Vec<String>,
        }
        let mut closure_fids: HashSet<FuncId> = HashSet::new();
        let mut facts = Vec::new();
        for f in &self.prog.funcs {
            let mut fa = Facts { throws: false, calls: Vec::new(), closure_calls: false, methods: Vec::new() };
            let mut throws = false;
            // Code inside a `try` with a `catch` cannot let an exception
            // out ("covered"), so it does not make the function throw.
            walk_covered(
                &f.body,
                false,
                &mut |s, covered| {
                    if !covered && matches!(s, TStmt::Throw(_)) {
                        throws = true;
                    }
                },
                &mut |e, covered| match &e.kind {
                    TExprKind::Closure(fid) => {
                        closure_fids.insert(*fid);
                    }
                    _ if covered => {}
                    TExprKind::Call(fid, _) => fa.calls.push(*fid),
                    TExprKind::CallClosure(..) | TExprKind::ArrayHof { .. } => fa.closure_calls = true,
                    TExprKind::MethodCall(_, name, ..) => fa.methods.push(name.clone()),
                    _ => {}
                },
            );
            fa.throws = throws;
            facts.push(fa);
        }
        loop {
            let mut changed = false;
            for (fid, fa) in facts.iter().enumerate() {
                let fid = fid as FuncId;
                if self.may_throw.contains(&fid) {
                    continue;
                }
                if fa.throws
                    || fa.calls.iter().any(|c| self.may_throw.contains(c))
                    || (fa.closure_calls && self.closures_throw)
                    || fa.methods.iter().any(|m| self.throwing_methods.contains(m))
                {
                    self.may_throw.insert(fid);
                    changed = true;
                }
            }
            let closures_throw = closure_fids.iter().any(|f| self.may_throw.contains(f));
            if closures_throw != self.closures_throw {
                self.closures_throw = closures_throw;
                changed = true;
            }
            for info in self.prog.classes.values() {
                for (name, fid) in &info.methods {
                    if self.may_throw.contains(fid) && self.throwing_methods.insert(name.clone()) {
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }

    fn literal_bytes(&self) -> Vec<u8> {
        self.literals.iter().flat_map(|s| s.bytes()).collect()
    }

    fn type_table_words(&self) -> usize {
        1 + self.user_types.iter().map(|t| 2 + t.ref_offsets.len()).sum::<usize>()
    }

    fn type_table_bytes(&self) -> Vec<u8> {
        let mut w: Vec<u32> = vec![self.user_types.len() as u32];
        for t in &self.user_types {
            w.push(t.size);
            w.push(t.ref_offsets.len() as u32);
            w.extend(&t.ref_offsets);
        }
        w.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    // -- Thunks -----------------------------------------------------------------

    /// The table index of the thunk for a closure signature.
    fn thunk(&mut self, sig: &ThunkSig) -> u32 {
        if let Some(i) = self.thunks.get(sig) {
            return *i;
        }
        let idx = self.reserve_func();
        let mut params = vec![ValType::I32];
        params.extend(sig.params.iter().filter_map(|r| r.val_type()));
        let call_ty = self.type_of(params, vt(sig.ret));
        let thunk_ty = self.type_of(vec![ValType::I32], vec![]);
        let rt = |n: &str| self.layout.rt(n);
        let mut f = Function::new(Vec::new());
        // The closure's env, then the argument, then the call.
        f.instruction(&I::LocalGet(0));
        f.instruction(&I::I32Load(mem(HEADER + 4, Repr::I32)));
        // The first argument is `arg`; the others (core 1.12) are `arg_at(i)`.
        for (i, r) in sig.params.iter().enumerate() {
            if i == 0 {
                f.instruction(&I::Call(if *r == Repr::F64 { rt("arg_f64") } else { rt("arg_i32") }));
            } else {
                f.instruction(&I::I32Const(i as i32));
                f.instruction(&I::Call(if *r == Repr::F64 { rt("arg_at_f64") } else { rt("arg_at_i32") }));
            }
        }
        f.instruction(&I::LocalGet(0));
        f.instruction(&I::I32Load(mem(HEADER, Repr::I32)));
        f.instruction(&I::CallIndirect { type_index: call_ty, table_index: 0 });
        if self.closures_throw {
            for i in self.report_uncaught() {
                f.instruction(&i);
            }
        }
        let rt = |n: &str| self.layout.rt(n);
        match sig.ret {
            Repr::F64 => {
                f.instruction(&I::Call(rt("ret_f64")));
            }
            Repr::I32 => {
                f.instruction(&I::Call(rt("ret_i32")));
            }
            Repr::Ref => {
                f.instruction(&I::Call(rt("ret_ref")));
            }
            Repr::Void => {}
        }
        f.instruction(&I::End);
        self.set_body(idx, thunk_ty, f);
        let slot = self.table_slot(idx);
        self.thunks.insert(sig.clone(), slot);
        slot
    }

    // -- Functions --------------------------------------------------------------

    fn compile(&mut self, fid: FuncId) {
        let def = &self.prog.funcs[fid as usize];
        let idx = self.func_index[&fid];
        let env = self.env_desc.get(&fid).copied().flatten();
        let mut fg = FnGen::new(self, fid, env);
        fg.ret = def.ret.repr();
        let mut params = vec![ValType::I32];
        for (i, v) in def.params.iter().enumerate() {
            let r = self.prog.vars[*v as usize].ty.repr();
            if let Some(t) = r.val_type() {
                params.push(t);
            }
            fg.var_local.insert(*v, i as u32 + 1);
        }
        fg.next_local = params.len() as u32;
        fg.prologue(self, def);
        for s in &def.body {
            fg.stmt(self, s);
        }
        if def.ret.repr() != Repr::Void {
            fg.emit(I::Unreachable);
        }
        fg.emit(I::End);
        let ty = self.type_of(params, vt(def.ret.repr()));
        let f = fg.finish();
        self.set_body(idx, ty, f);
    }
}

/// Collects the variables that statements read or write, without entering
/// closures (which are other functions).
fn stmt_vars(s: &TStmt, out: &mut Vec<VarId>) {
    match s {
        TStmt::Let(v, e) => {
            out.push(*v);
            if let Some(e) = e {
                expr_vars(e, out);
            }
        }
        TStmt::Expr(e) | TStmt::Return(Some(e)) | TStmt::Throw(e) | TStmt::Trap(e) => expr_vars(e, out),
        TStmt::Try { body, catch, finally } => {
            if let Some((v, _)) = catch {
                out.push(*v);
            }
            let catch_body = catch.iter().flat_map(|(_, b)| b.iter());
            for x in body.iter().chain(catch_body).chain(finally.iter().flatten()) {
                stmt_vars(x, out);
            }
        }
        TStmt::If(c, a, b) => {
            expr_vars(c, out);
            for x in a.iter().chain(b) {
                stmt_vars(x, out);
            }
        }
        TStmt::Loop { cond, update, body, .. } => {
            if let Some(c) = cond {
                expr_vars(c, out);
            }
            if let Some(u) = update {
                expr_vars(u, out);
            }
            for x in body {
                stmt_vars(x, out);
            }
        }
        TStmt::ForOf { var, arr, body, .. } => {
            out.push(*var);
            expr_vars(arr, out);
            for x in body {
                stmt_vars(x, out);
            }
        }
        TStmt::Switch { disc, cases, .. } => {
            expr_vars(disc, out);
            for (t, b) in cases {
                if let Some(t) = t {
                    expr_vars(t, out);
                }
                for x in b {
                    stmt_vars(x, out);
                }
            }
        }
        TStmt::Block(b) => {
            for x in b {
                stmt_vars(x, out);
            }
        }
        TStmt::Return(None) | TStmt::Break | TStmt::Continue => {}
    }
}

fn expr_vars(e: &TExpr, out: &mut Vec<VarId>) {
    let mut go = |x: &TExpr| expr_vars(x, out);
    match &e.kind {
        TExprKind::Var(v) => out.push(*v),
        TExprKind::Assign(p, v) => {
            match p {
                Place::Var(x) => out.push(*x),
                Place::Field(o, ..) => expr_vars(o, out),
                Place::Index(a, i) => {
                    expr_vars(a, out);
                    expr_vars(i, out);
                }
            }
            expr_vars(v, out);
        }
        TExprKind::Field(o, ..)
        | TExprKind::Neg(o)
        | TExprKind::Not(o)
        | TExprKind::IsNull(o)
        | TExprKind::Coerce(_, o)
        | TExprKind::UnionTag(o)
        | TExprKind::UnionIs(o, _) => go(o),
        TExprKind::Index(a, b)
        | TExprKind::Num2(_, a, b)
        | TExprKind::Int2(_, a, b)
        | TExprKind::Cmp(_, _, a, b)
        | TExprKind::StrCmp(_, a, b)
        | TExprKind::Concat(a, b)
        | TExprKind::And(a, b)
        | TExprKind::Or(a, b) => {
            go(a);
            go(b);
        }
        TExprKind::Cond(a, b, c) => {
            go(a);
            go(b);
            go(c);
        }
        TExprKind::Call(_, args) | TExprKind::Rt(_, args) | TExprKind::MathOp(_, args) | TExprKind::StructLit(_, args) => {
            for a in args {
                expr_vars(a, out);
            }
        }
        TExprKind::CallClosure(c, args) => {
            expr_vars(c, out);
            for a in args {
                expr_vars(a, out);
            }
        }
        TExprKind::ArrayLit(items) => {
            for (_, a) in items {
                expr_vars(a, out);
            }
        }
        TExprKind::ArrayHof { arr, f, .. } => {
            expr_vars(arr, out);
            expr_vars(f, out);
        }
        TExprKind::ArraySearch { arr, value, .. } => {
            expr_vars(arr, out);
            expr_vars(value, out);
        }
        TExprKind::Block(stmts, v) => {
            for s in stmts {
                stmt_vars(s, out);
            }
            expr_vars(v, out);
        }
        TExprKind::MethodCall(_, _, this, args) => {
            expr_vars(this, out);
            for a in args {
                expr_vars(a, out);
            }
        }
        TExprKind::InstanceOf(o, _) => go(o),
        _ => {}
    }
}

// ---------------------------------------------------------------------------

struct Label {
    /// The block depth of the `break` target.
    brk: u32,
    /// The block depth of the `continue` target, if this is a loop.
    cont: Option<u32>,
}

/// The code generator state of one function.
struct FnGen {
    fid: FuncId,
    code: Vec<I<'static>>,
    locals: Vec<ValType>,
    next_local: u32,
    var_local: HashMap<VarId, u32>,
    /// Active frames of this function: `(desc, local holding the pointer)`.
    frames: Vec<(usize, u32)>,
    /// The frame that local 0 points to.
    env: Option<usize>,
    depth: u32,
    labels: Vec<Label>,
    scratch_i32: Vec<u32>,
    scratch_f64: Vec<u32>,
    /// The function's return representation, for an early `return` when an
    /// exception leaves the function.
    ret: Repr,
    /// The block depths of the enclosing exception handlers (`try`
    /// regions); an exception branches to the innermost one.
    handlers: Vec<u32>,
    /// The enclosing `finally` blocks.
    finallies: Vec<FinallyCx>,
}

impl FnGen {
    fn new(_g: &Codegen, fid: FuncId, env: Option<usize>) -> Self {
        FnGen {
            fid,
            code: Vec::new(),
            locals: Vec::new(),
            next_local: 0,
            var_local: HashMap::new(),
            frames: Vec::new(),
            env,
            depth: 0,
            labels: Vec::new(),
            scratch_i32: Vec::new(),
            scratch_f64: Vec::new(),
            ret: Repr::Void,
            handlers: Vec::new(),
            finallies: Vec::new(),
        }
    }

    // -- Exceptions (SPEC.md §5.6) ------------------------------------------

    /// Leaves for the innermost handler, or returns from the function (with
    /// a zero value) when there is none: the caller checks the flag.
    fn propagate(&mut self) {
        match self.handlers.last() {
            Some(&d) => self.emit(I::Br(self.depth - d)),
            None => {
                self.zero(self.ret);
                self.emit(I::Return);
            }
        }
    }

    /// After a call that can throw: propagate a pending exception.
    fn check_exc(&mut self, g: &mut Codegen) {
        let (flag, _) = g.exc_globals();
        self.emit(I::GlobalGet(flag));
        self.open(I::If(BlockType::Empty));
        self.propagate();
        self.close();
    }

    /// Runs the `finally` blocks from index `from` outward (innermost
    /// first), for a jump out of them. Each one runs with the handlers and
    /// the `finally` blocks outside its own `try`.
    fn run_finallies(&mut self, g: &mut Codegen, from: usize) {
        let saved_f = self.finallies.clone();
        let saved_h = self.handlers.clone();
        for i in (from..saved_f.len()).rev() {
            self.finallies.truncate(i);
            self.handlers.truncate(saved_f[i].handlers);
            let body = saved_f[i].body.clone();
            self.stmts(g, &body);
        }
        self.finallies = saved_f;
        self.handlers = saved_h;
    }

    /// Before a jump to label `idx`: runs the `finally` blocks of the `try`
    /// statements that the jump leaves.
    fn leave_finallies(&mut self, g: &mut Codegen, idx: usize) {
        if let Some(from) = self.finallies.iter().position(|f| f.labels > idx) {
            self.run_finallies(g, from);
        }
    }

    fn try_stmt(&mut self, g: &mut Codegen, body: &[TStmt], catch: &Option<(VarId, Vec<TStmt>)>, finally: &Option<Vec<TStmt>>) {
        let (flag, val) = g.exc_globals();
        let outer = self.handlers.len();
        self.open(I::Block(BlockType::Empty));
        let done = self.depth;
        let exc = finally.as_ref().map(|_| {
            self.open(I::Block(BlockType::Empty));
            self.depth
        });
        let catch_l = catch.as_ref().map(|_| {
            self.open(I::Block(BlockType::Empty));
            self.depth
        });
        let fin_cx = |s: &Self, f: &Vec<TStmt>| FinallyCx { body: f.clone(), labels: s.labels.len(), handlers: outer };
        // The `try` body.
        self.handlers.push(catch_l.or(exc).expect("a try has a catch or a finally"));
        if let Some(f) = finally {
            let cx = fin_cx(self, f);
            self.finallies.push(cx);
        }
        self.stmts(g, body);
        if finally.is_some() {
            self.finallies.pop();
        }
        self.handlers.pop();
        if let Some(f) = finally {
            self.stmts(g, f);
        }
        self.emit(I::Br(self.depth - done));
        // The `catch` body: the exception is caught, so clear it.
        if let Some((v, cbody)) = catch {
            self.close();
            self.emit(I::GlobalGet(val));
            self.set_var(g, *v);
            self.emit(I::I32Const(0));
            self.emit(I::GlobalSet(flag));
            if let (Some(e), Some(f)) = (exc, finally) {
                self.handlers.push(e);
                let cx = fin_cx(self, f);
                self.finallies.push(cx);
            }
            self.stmts(g, cbody);
            if exc.is_some() {
                self.finallies.pop();
                self.handlers.pop();
            }
            if let Some(f) = finally {
                self.stmts(g, f);
            }
            self.emit(I::Br(self.depth - done));
        }
        // An exception left the `try` (or the `catch`): run the `finally`
        // block with the exception put aside, then throw it again.
        if let Some(f) = finally {
            self.close();
            let t = self.local(ValType::I32);
            self.emit(I::GlobalGet(val));
            self.emit(I::LocalSet(t));
            self.emit(I::I32Const(0));
            self.emit(I::GlobalSet(flag));
            self.stmts(g, f);
            self.emit(I::LocalGet(t));
            self.emit(I::GlobalSet(val));
            self.emit(I::I32Const(1));
            self.emit(I::GlobalSet(flag));
            self.propagate();
        }
        self.close();
    }

    fn finish(self) -> Function {
        let locals: Vec<(u32, ValType)> = self.locals.iter().map(|t| (1, *t)).collect();
        let mut f = Function::new(locals);
        for i in &self.code {
            f.instruction(i);
        }
        f
    }

    fn emit(&mut self, i: I<'static>) {
        self.code.push(i);
    }

    fn local(&mut self, t: ValType) -> u32 {
        self.locals.push(t);
        self.next_local += 1;
        self.next_local - 1
    }

    /// A temporary local; give it back with `free`.
    fn tmp(&mut self, r: Repr) -> u32 {
        let pool = if r == Repr::F64 { &mut self.scratch_f64 } else { &mut self.scratch_i32 };
        if let Some(l) = pool.pop() {
            return l;
        }
        self.local(if r == Repr::F64 { ValType::F64 } else { ValType::I32 })
    }

    fn free(&mut self, l: u32, r: Repr) {
        if r == Repr::F64 { self.scratch_f64.push(l) } else { self.scratch_i32.push(l) }
    }

    fn open(&mut self, i: I<'static>) {
        self.emit(i);
        self.depth += 1;
    }

    fn close(&mut self) {
        self.emit(I::End);
        self.depth -= 1;
    }

    fn rt(&mut self, g: &Codegen, name: &str) {
        self.emit(I::Call(g.layout.rt(name)));
    }

    // -- Frames -------------------------------------------------------------

    /// The captured variables that `fid` declares directly in the frame of
    /// loop `l` (or its root frame).
    fn frame_vars(g: &Codegen, fid: FuncId, l: Option<LoopId>) -> Vec<VarId> {
        let mut vars: Vec<VarId> = g
            .captured
            .iter()
            .copied()
            .filter(|v| {
                let i = &g.prog.vars[*v as usize];
                i.owner == fid && i.in_loop == l
            })
            .collect();
        vars.sort();
        vars
    }

    /// Allocates a frame for `(fid, l)` if it has captured variables.
    fn push_frame(&mut self, g: &mut Codegen, l: Option<LoopId>) -> bool {
        let vars = Self::frame_vars(g, self.fid, l);
        if vars.is_empty() {
            return false;
        }
        let parent = self.frames.last().map(|(d, _)| *d).or(self.env);
        let key = (self.fid, l);
        let desc = match g.frame_by_key.get(&key) {
            Some(d) => *d,
            None => g.new_frame(key, parent, &vars, true),
        };
        let local = self.local(ValType::I32);
        self.emit(I::I32Const(g.frames[desc].type_id as i32));
        self.rt(g, "alloc");
        self.emit(I::LocalSet(local));
        // The parent pointer.
        self.emit(I::LocalGet(local));
        self.current_frame_ptr();
        self.emit(I::I32Store(mem(HEADER, Repr::I32)));
        self.frames.push((desc, local));
        true
    }

    /// Pushes the innermost frame pointer (or the env, or 0).
    fn current_frame_ptr(&mut self) {
        match self.frames.last() {
            Some((_, l)) => {
                let l = *l;
                self.emit(I::LocalGet(l))
            }
            None => self.emit(I::LocalGet(0)),
        }
    }

    fn current_desc(&self) -> Option<usize> {
        self.frames.last().map(|(d, _)| *d).or(self.env)
    }

    fn prologue(&mut self, g: &mut Codegen, def: &FuncDef) {
        if self.push_frame(g, None) {
            // Captured parameters move into the frame.
            for v in &def.params {
                if g.captured.contains(v) {
                    let l = self.var_local[v];
                    let (local, (off, r)) = self.slot_of(g, *v).expect("a captured parameter is in the root frame");
                    self.emit(I::LocalGet(local));
                    self.emit(I::LocalGet(l));
                    self.emit(store(off, r));
                }
            }
        }
    }

    /// The frame local and slot of a variable of this function.
    fn slot_of(&self, g: &Codegen, v: VarId) -> Option<(u32, (u32, Repr))> {
        for (desc, local) in self.frames.iter().rev() {
            if let Some(s) = g.frames[*desc].slots.get(&v) {
                return Some((*local, *s));
            }
        }
        None
    }

    /// Pushes the address base for variable `v` and returns its slot. The
    /// value is at `base + offset`.
    fn var_base(&mut self, g: &Codegen, v: VarId) -> (u32, Repr) {
        let info = &g.prog.vars[v as usize];
        if let Some(m) = info.module {
            let (global, desc) = g.module_env[&m];
            self.emit(I::GlobalGet(global));
            return g.frames[desc].slots[&v];
        }
        if let Some((local, slot)) = self.slot_of(g, v) {
            self.emit(I::LocalGet(local));
            return slot;
        }
        // An outer variable: walk up from the env.
        let key = (info.owner, info.in_loop);
        let target = *g.frame_by_key.get(&key).unwrap_or_else(|| panic!("no frame for captured `{}`", info.name));
        self.emit(I::LocalGet(0));
        let mut d = self.env.unwrap_or_else(|| panic!("`{}` is captured, but the function has no env", info.name));
        while d != target {
            self.emit(I::I32Load(mem(HEADER, Repr::I32)));
            d = g.frames[d].parent.unwrap_or_else(|| panic!("`{}` is not in the env chain", info.name));
        }
        g.frames[target].slots[&v]
    }

    fn is_local(&self, g: &Codegen, v: VarId) -> bool {
        g.prog.vars[v as usize].module.is_none() && !g.captured.contains(&v)
    }

    fn get_var(&mut self, g: &Codegen, v: VarId) {
        if self.is_local(g, v) {
            let l = self.local_of(g, v);
            self.emit(I::LocalGet(l));
            return;
        }
        let (off, r) = self.var_base(g, v);
        self.emit(load(off, r));
    }

    fn local_of(&mut self, g: &Codegen, v: VarId) -> u32 {
        if let Some(l) = self.var_local.get(&v) {
            return *l;
        }
        let t = g.prog.vars[v as usize].ty.repr().val_type().unwrap_or(ValType::I32);
        let l = self.local(t);
        self.var_local.insert(v, l);
        l
    }

    /// Stores the value on the stack into `v`.
    fn set_var(&mut self, g: &Codegen, v: VarId) {
        if self.is_local(g, v) {
            let l = self.local_of(g, v);
            self.emit(I::LocalSet(l));
            return;
        }
        let r = g.prog.vars[v as usize].ty.repr();
        let t = self.tmp(r);
        self.emit(I::LocalSet(t));
        let (off, r) = self.var_base(g, v);
        self.emit(I::LocalGet(t));
        self.emit(store(off, r));
        self.free(t, r);
    }

    fn zero(&mut self, r: Repr) {
        match r {
            Repr::F64 => self.emit(I::F64Const(0.0.into())),
            Repr::Void => {}
            _ => self.emit(I::I32Const(0)),
        }
    }

    // -- Statements ---------------------------------------------------------

    fn stmts(&mut self, g: &mut Codegen, stmts: &[TStmt]) {
        for s in stmts {
            self.stmt(g, s);
        }
    }

    fn stmt(&mut self, g: &mut Codegen, s: &TStmt) {
        match s {
            TStmt::Let(v, init) => {
                match init {
                    Some(e) => self.expr(g, e),
                    None => self.zero(g.prog.vars[*v as usize].ty.repr()),
                }
                self.set_var(g, *v);
            }
            TStmt::Expr(e) => {
                self.expr(g, e);
                if e.repr() != Repr::Void {
                    self.emit(I::Drop);
                }
            }
            TStmt::If(c, a, b) => {
                self.expr(g, c);
                self.open(I::If(BlockType::Empty));
                self.stmts(g, a);
                if !b.is_empty() {
                    self.emit(I::Else);
                    self.stmts(g, b);
                }
                self.close();
            }
            TStmt::Block(b) => self.stmts(g, b),
            TStmt::Loop { id, cond, test_after, update, body } => {
                self.open(I::Block(BlockType::Empty));
                let brk = self.depth;
                self.open(I::Loop(BlockType::Empty));
                let top = self.depth;
                if let (Some(c), false) = (cond, test_after) {
                    self.expr(g, c);
                    self.emit(I::I32Eqz);
                    self.emit(I::BrIf(self.depth - brk));
                }
                let frames_before = self.frames.len();
                self.push_frame(g, Some(*id));
                self.open(I::Block(BlockType::Empty));
                let cont = self.depth;
                self.labels.push(Label { brk, cont: Some(cont) });
                self.stmts(g, body);
                self.labels.pop();
                self.close();
                self.frames.truncate(frames_before);
                if let Some(u) = update {
                    self.expr(g, u);
                    if u.repr() != Repr::Void {
                        self.emit(I::Drop);
                    }
                }
                match (cond, test_after) {
                    (Some(c), true) => {
                        self.expr(g, c);
                        self.emit(I::BrIf(self.depth - top));
                    }
                    _ => self.emit(I::Br(self.depth - top)),
                }
                self.close();
                self.close();
            }
            TStmt::ForOf { id, var, arr, body } => {
                let elem_r = g.prog.vars[*var as usize].ty.repr();
                let (ta, tn, ti) = (self.local(ValType::I32), self.local(ValType::I32), self.local(ValType::I32));
                self.expr(g, arr);
                self.emit(I::LocalTee(ta));
                self.rt(g, "arr_len");
                self.emit(I::LocalSet(tn));
                self.emit(I::I32Const(0));
                self.emit(I::LocalSet(ti));
                self.open(I::Block(BlockType::Empty));
                let brk = self.depth;
                self.open(I::Loop(BlockType::Empty));
                let top = self.depth;
                self.emit(I::LocalGet(ti));
                self.emit(I::LocalGet(tn));
                self.emit(I::I32GeS);
                self.emit(I::BrIf(self.depth - brk));
                let frames_before = self.frames.len();
                self.push_frame(g, Some(*id));
                self.emit(I::LocalGet(ta));
                self.emit(I::LocalGet(ti));
                self.rt(g, if elem_r == Repr::F64 { "arr_get_f64" } else { "arr_get_i32" });
                self.set_var(g, *var);
                self.open(I::Block(BlockType::Empty));
                let cont = self.depth;
                self.labels.push(Label { brk, cont: Some(cont) });
                self.stmts(g, body);
                self.labels.pop();
                self.close();
                self.frames.truncate(frames_before);
                self.emit(I::LocalGet(ti));
                self.emit(I::I32Const(1));
                self.emit(I::I32Add);
                self.emit(I::LocalSet(ti));
                self.emit(I::Br(self.depth - top));
                self.close();
                self.close();
            }
            TStmt::Return(e) => {
                if self.finallies.is_empty() {
                    if let Some(e) = e {
                        self.expr(g, e);
                    }
                } else {
                    // Keep the value while the `finally` blocks run.
                    let t = e.as_ref().and_then(|e| {
                        self.expr(g, e);
                        let t = e.repr().val_type().map(|vt| self.local(vt));
                        if let Some(t) = t {
                            self.emit(I::LocalSet(t));
                        }
                        t
                    });
                    self.run_finallies(g, 0);
                    if let Some(t) = t {
                        self.emit(I::LocalGet(t));
                    }
                }
                self.emit(I::Return);
            }
            TStmt::Break => {
                let idx = self.labels.len() - 1;
                self.leave_finallies(g, idx);
                let l = self.labels.last().expect("break inside a loop or switch").brk;
                self.emit(I::Br(self.depth - l));
            }
            TStmt::Continue => {
                let idx = self.labels.iter().rposition(|l| l.cont.is_some()).expect("continue inside a loop");
                self.leave_finallies(g, idx);
                let l = self.labels[idx].cont.expect("a loop label");
                self.emit(I::Br(self.depth - l));
            }
            TStmt::Throw(e) => {
                let (flag, val) = g.exc_globals();
                self.expr(g, e);
                self.emit(I::GlobalSet(val));
                self.emit(I::I32Const(1));
                self.emit(I::GlobalSet(flag));
                self.propagate();
            }
            TStmt::Trap(e) => {
                self.expr(g, e);
                self.rt(g, "throw");
                self.emit(I::Unreachable);
            }
            TStmt::Try { body, catch, finally } => self.try_stmt(g, body, catch, finally),
            TStmt::Switch { disc, eq, cases } => self.switch(g, disc, *eq, cases),
        }
    }

    fn switch(&mut self, g: &mut Codegen, disc: &TExpr, eq: EqKind, cases: &[(Option<TExpr>, Vec<TStmt>)]) {
        let r = disc.repr();
        let td = self.local(r.val_type().unwrap_or(ValType::I32));
        let k = self.local(ValType::I32);
        self.expr(g, disc);
        self.emit(I::LocalSet(td));
        let n = cases.len() as i32;
        let default = cases.iter().position(|(t, _)| t.is_none()).map(|i| i as i32).unwrap_or(n);
        // Select the case index.
        self.open(I::Block(BlockType::Empty));
        let sel = self.depth;
        for (i, (t, _)) in cases.iter().enumerate() {
            let Some(t) = t else { continue };
            self.emit(I::LocalGet(td));
            self.expr(g, t);
            self.eq(g, eq);
            self.open(I::If(BlockType::Empty));
            self.emit(I::I32Const(i as i32));
            self.emit(I::LocalSet(k));
            self.emit(I::Br(self.depth - sel));
            self.close();
        }
        self.emit(I::I32Const(default));
        self.emit(I::LocalSet(k));
        self.close();
        // Dispatch with fallthrough.
        self.open(I::Block(BlockType::Empty));
        let exit = self.depth;
        for _ in 0..cases.len() {
            self.open(I::Block(BlockType::Empty));
        }
        self.emit(I::LocalGet(k));
        let targets: Vec<u32> = (0..cases.len() as u32).collect();
        self.emit(I::BrTable(targets.into(), cases.len() as u32));
        self.labels.push(Label { brk: exit, cont: None });
        for (_, body) in cases {
            self.close();
            self.stmts(g, body);
        }
        self.labels.pop();
        self.close();
    }

    /// Compares the two values on the stack.
    fn eq(&mut self, g: &Codegen, eq: EqKind) {
        match eq {
            EqKind::F64 => self.emit(I::F64Eq),
            EqKind::I32 | EqKind::Ref => self.emit(I::I32Eq),
            EqKind::Str => self.rt(g, "str_eq"),
            EqKind::NullStr | EqKind::NullF64 | EqKind::NullI32 => {
                // a !== null && unwrap(a) === b
                let br = match eq {
                    EqKind::NullF64 => Repr::F64,
                    EqKind::NullI32 => Repr::I32,
                    _ => Repr::I32,
                };
                let (ta, tb) = (self.tmp(Repr::I32), self.tmp(br));
                self.emit(I::LocalSet(tb));
                self.emit(I::LocalTee(ta));
                self.open(I::If(BlockType::Result(ValType::I32)));
                self.emit(I::LocalGet(ta));
                if eq == EqKind::NullF64 || eq == EqKind::NullI32 {
                    self.rt(g, "unbox_f64");
                }
                if eq == EqKind::NullI32 {
                    self.emit(I::I32TruncSatF64S);
                }
                self.emit(I::LocalGet(tb));
                match eq {
                    EqKind::NullF64 => self.emit(I::F64Eq),
                    EqKind::NullI32 => self.emit(I::I32Eq),
                    _ => self.rt(g, "str_eq"),
                }
                self.emit(I::Else);
                self.emit(I::I32Const(0));
                self.close();
                self.free(ta, Repr::I32);
                self.free(tb, br);
            }
        }
    }

    // -- Expressions --------------------------------------------------------

    fn expr(&mut self, g: &mut Codegen, e: &TExpr) {
        let r = e.repr();
        match &e.kind {
            TExprKind::Num(v) => match r {
                Repr::F64 => self.emit(I::F64Const((*v).into())),
                _ => self.emit(I::I32Const(*v as i32)),
            },
            TExprKind::Bool(b) => self.emit(I::I32Const(*b as i32)),
            TExprKind::Str(s) => {
                let gl = g.literal(s);
                self.emit(I::GlobalGet(gl));
            }
            TExprKind::Null => self.zero(if r == Repr::F64 { Repr::F64 } else { Repr::I32 }),
            TExprKind::Var(v) => self.get_var(g, *v),
            TExprKind::Assign(place, value) => self.assign(g, place, value),
            TExprKind::Field(o, sid, idx) => {
                self.expr(g, o);
                let (off, fr) = g.structs[*sid as usize].offsets[*idx as usize];
                self.emit(load(off, fr));
            }
            TExprKind::Index(a, i) => {
                self.expr(g, a);
                self.expr(g, i);
                self.emit(I::I32TruncSatF64S);
                self.rt(g, if r == Repr::F64 { "arr_get_f64" } else { "arr_get_i32" });
            }
            TExprKind::Call(fid, args) => {
                self.emit(I::I32Const(0));
                for a in args {
                    self.expr(g, a);
                }
                let idx = g.func_ref(*fid);
                self.emit(I::Call(idx));
                if g.may_throw.contains(fid) {
                    self.check_exc(g);
                }
            }
            TExprKind::CallClosure(c, args) => {
                let Type::Func(ft) = &c.ty else { panic!("calling a non-function") };
                let ty = g.sig_type(ft);
                let t = self.tmp(Repr::I32);
                self.expr(g, c);
                self.emit(I::LocalTee(t));
                self.emit(I::I32Load(mem(HEADER + 4, Repr::I32)));
                for a in args {
                    self.expr(g, a);
                }
                self.emit(I::LocalGet(t));
                self.emit(I::I32Load(mem(HEADER, Repr::I32)));
                self.free(t, Repr::I32);
                self.emit(I::CallIndirect { type_index: ty, table_index: 0 });
                if g.closures_throw {
                    self.check_exc(g);
                }
            }
            TExprKind::Closure(fid) => self.closure(g, *fid),
            TExprKind::Num2(op, a, b) => {
                self.expr(g, a);
                self.expr(g, b);
                match op {
                    NumOp::Add => self.emit(I::F64Add),
                    NumOp::Sub => self.emit(I::F64Sub),
                    NumOp::Mul => self.emit(I::F64Mul),
                    NumOp::Div => self.emit(I::F64Div),
                    NumOp::Rem => self.rt(g, "f64_rem"),
                    NumOp::Pow => self.rt(g, "f64_pow"),
                }
            }
            TExprKind::Int2(op, a, b) => {
                self.expr(g, a);
                self.expr(g, b);
                match op {
                    IntOp::Add => self.emit(I::I32Add),
                    IntOp::Sub => self.emit(I::I32Sub),
                    IntOp::Mul => self.emit(I::I32Mul),
                    IntOp::Div => self.emit(I::I32DivS),
                    IntOp::Rem => self.emit(I::I32RemS),
                }
            }
            TExprKind::Neg(a) => {
                self.expr(g, a);
                self.emit(I::F64Neg);
            }
            TExprKind::Not(a) => {
                self.expr(g, a);
                self.emit(I::I32Eqz);
            }
            TExprKind::Cmp(op, kind, a, b) => {
                self.expr(g, a);
                self.expr(g, b);
                match kind {
                    EqKind::F64 => self.emit(match op {
                        CmpOp::Eq => I::F64Eq,
                        CmpOp::Ne => I::F64Ne,
                        CmpOp::Lt => I::F64Lt,
                        CmpOp::Le => I::F64Le,
                        CmpOp::Gt => I::F64Gt,
                        CmpOp::Ge => I::F64Ge,
                    }),
                    EqKind::Str | EqKind::NullStr | EqKind::NullF64 | EqKind::NullI32 => {
                        self.eq(g, *kind);
                        if *op == CmpOp::Ne {
                            self.emit(I::I32Eqz);
                        }
                    }
                    EqKind::I32 | EqKind::Ref => self.emit(match op {
                        CmpOp::Ne => I::I32Ne,
                        CmpOp::Lt => I::I32LtS,
                        CmpOp::Le => I::I32LeS,
                        CmpOp::Gt => I::I32GtS,
                        CmpOp::Ge => I::I32GeS,
                        CmpOp::Eq => I::I32Eq,
                    }),
                }
            }
            TExprKind::StrCmp(op, a, b) => {
                self.expr(g, a);
                self.expr(g, b);
                self.rt(g, "str_cmp");
                self.emit(I::I32Const(0));
                self.emit(match op {
                    CmpOp::Lt => I::I32LtS,
                    CmpOp::Le => I::I32LeS,
                    CmpOp::Gt => I::I32GtS,
                    CmpOp::Ge => I::I32GeS,
                    CmpOp::Eq => I::I32Eq,
                    CmpOp::Ne => I::I32Ne,
                });
            }
            TExprKind::Concat(a, b) => {
                self.expr(g, a);
                self.expr(g, b);
                self.rt(g, "str_concat");
            }
            TExprKind::And(a, b) => {
                self.expr(g, a);
                self.open(I::If(BlockType::Result(ValType::I32)));
                self.expr(g, b);
                self.emit(I::Else);
                self.emit(I::I32Const(0));
                self.close();
            }
            TExprKind::Or(a, b) => {
                self.expr(g, a);
                self.open(I::If(BlockType::Result(ValType::I32)));
                self.emit(I::I32Const(1));
                self.emit(I::Else);
                self.expr(g, b);
                self.close();
            }
            TExprKind::Cond(c, a, b) => {
                self.expr(g, c);
                self.open(I::If(block_type(r)));
                self.expr(g, a);
                self.emit(I::Else);
                self.expr(g, b);
                self.close();
            }
            TExprKind::IsNull(a) => {
                self.expr(g, a);
                self.emit(I::I32Eqz);
            }
            TExprKind::Coerce(c, a) => self.coerce(g, *c, a),
            TExprKind::Block(stmts, v) => {
                self.stmts(g, stmts);
                self.expr(g, v);
            }
            TExprKind::ArrayLit(items) => {
                let Type::Array(elem) = &e.ty else { panic!("array literal without an array type") };
                let er = elem.repr();
                let t = self.local(ValType::I32);
                self.emit(I::I32Const(arr_kind(er)));
                self.emit(I::I32Const(items.len() as i32));
                self.rt(g, "arr_new");
                self.emit(I::LocalSet(t));
                for (spread, x) in items {
                    self.emit(I::LocalGet(t));
                    self.expr(g, x);
                    if *spread {
                        self.rt(g, "arr_extend");
                    } else {
                        self.rt(g, if er == Repr::F64 { "arr_push_f64" } else { "arr_push_i32" });
                        self.emit(I::Drop);
                    }
                }
                self.emit(I::LocalGet(t));
            }
            TExprKind::StructLit(sid, values) => {
                let t = self.local(ValType::I32);
                let layout_type = g.sid_type(*sid);
                self.emit(I::I32Const(layout_type as i32));
                self.rt(g, "alloc");
                self.emit(I::LocalSet(t));
                for (i, v) in values.iter().enumerate() {
                    let (off, fr) = g.structs[*sid as usize].offsets[i];
                    self.emit(I::LocalGet(t));
                    self.expr(g, v);
                    self.emit(store(off, fr));
                }
                self.emit(I::LocalGet(t));
            }
            TExprKind::ArrayHof { kind, arr, f, arity } => self.array_hof(g, *kind, arr, f, *arity, &e.ty),
            TExprKind::ArraySearch { index, eq, arr, value } => self.array_search(g, *index, *eq, arr, value),
            TExprKind::Rt(name, args) => {
                for a in args {
                    self.expr(g, a);
                }
                self.rt(g, name);
            }
            TExprKind::MathOp(op, args) => {
                for a in args {
                    self.expr(g, a);
                }
                self.emit(match op {
                    MathOp::Floor => I::F64Floor,
                    MathOp::Ceil => I::F64Ceil,
                    MathOp::Trunc => I::F64Trunc,
                    MathOp::Abs => I::F64Abs,
                    MathOp::Sqrt => I::F64Sqrt,
                    MathOp::Min => I::F64Min,
                    MathOp::Max => I::F64Max,
                });
            }
            TExprKind::ThunkOf(sig) => {
                let slot = g.thunk(sig);
                for i in table_ref(g.layout, slot) {
                    self.emit(i);
                }
            }
            TExprKind::UnionTag(o) => {
                // The discriminant is the first field of every member
                // struct, so it is always at the header offset.
                self.expr(g, o);
                self.emit(load(HEADER, Repr::Ref));
            }
            TExprKind::UnionIs(o, idxs) => {
                let Type::Union(members) = &o.ty else { panic!("UnionIs on a non-union") };
                let ids: Vec<u32> = idxs.iter().map(|i| g.type_id_of(&members[*i])).collect();
                self.expr(g, o);
                self.emit(load(0, Repr::I32));
                let th = self.tmp(Repr::I32);
                self.emit(I::LocalSet(th));
                for (i, id) in ids.iter().enumerate() {
                    self.emit(I::LocalGet(th));
                    self.emit(I::I32Const(*id as i32));
                    self.emit(I::I32Eq);
                    if i > 0 {
                        self.emit(I::I32Or);
                    }
                }
                self.free(th, Repr::I32);
            }
            TExprKind::MethodCall(sid, name, this, args) => {
                let targets = g.dispatch_targets(*sid, name);
                if targets.len() == 1 {
                    self.emit(I::I32Const(0));
                    self.expr(g, this);
                    for a in args {
                        self.expr(g, a);
                    }
                    self.emit(I::Call(targets[0].1));
                } else {
                    let this_t = self.tmp(Repr::I32);
                    self.expr(g, this);
                    self.emit(I::LocalTee(this_t));
                    self.emit(load(0, Repr::I32));
                    let tag_t = self.tmp(Repr::I32);
                    self.emit(I::LocalSet(tag_t));
                    let mut arg_temps = Vec::new();
                    for a in args {
                        let ar = a.repr();
                        let t = self.tmp(ar);
                        self.expr(g, a);
                        self.emit(I::LocalSet(t));
                        arg_temps.push((t, ar));
                    }
                    self.emit_dispatch(&targets, tag_t, this_t, &arg_temps, block_type(r));
                    self.free(this_t, Repr::I32);
                    self.free(tag_t, Repr::I32);
                    for (t, ar) in arg_temps {
                        self.free(t, ar);
                    }
                }
                if g.throwing_methods.contains(name) {
                    self.check_exc(g);
                }
            }
            TExprKind::InstanceOf(o, sid) => {
                let ids: Vec<u32> = g.class_descendants(*sid).iter().map(|s| g.sid_type(*s)).collect();
                self.expr(g, o);
                self.emit(load(0, Repr::I32));
                let th = self.tmp(Repr::I32);
                self.emit(I::LocalSet(th));
                for (i, id) in ids.iter().enumerate() {
                    self.emit(I::LocalGet(th));
                    self.emit(I::I32Const(*id as i32));
                    self.emit(I::I32Eq);
                    if i > 0 {
                        self.emit(I::I32Or);
                    }
                }
                self.free(th, Repr::I32);
            }
            other => panic!("codegen: `lower` must remove {other:?}"),
        }
    }

    /// Emits a chain of `if type_id == … { call … } else { … }`, ending
    /// (for the last target) in a plain call with no check (SPEC.md §4.2
    /// v1 overriding).
    fn emit_dispatch(&mut self, targets: &[(u32, u32)], tag_t: u32, this_t: u32, arg_temps: &[(u32, Repr)], bt: BlockType) {
        if targets.len() == 1 {
            self.emit(I::I32Const(0));
            self.emit(I::LocalGet(this_t));
            for (t, _) in arg_temps {
                self.emit(I::LocalGet(*t));
            }
            self.emit(I::Call(targets[0].1));
            return;
        }
        self.emit(I::LocalGet(tag_t));
        self.emit(I::I32Const(targets[0].0 as i32));
        self.emit(I::I32Eq);
        self.open(I::If(bt));
        self.emit(I::I32Const(0));
        self.emit(I::LocalGet(this_t));
        for (t, _) in arg_temps {
            self.emit(I::LocalGet(*t));
        }
        self.emit(I::Call(targets[0].1));
        self.emit(I::Else);
        self.emit_dispatch(&targets[1..], tag_t, this_t, arg_temps, bt);
        self.close();
    }

    fn closure(&mut self, g: &mut Codegen, fid: FuncId) {
        let def = &g.prog.funcs[fid as usize];
        let idx = g.func_ref(fid);
        let slot = g.table_slot(idx);
        for i in table_ref(g.layout, slot) {
            self.emit(i);
        }
        if def.kind == FuncKind::Closure {
            let desc = self.current_desc();
            match g.env_desc.get(&fid) {
                Some(prev) if *prev != desc => panic!("closure {} is created in two places", def.name),
                _ => {}
            }
            g.env_desc.insert(fid, desc);
            self.current_frame_ptr();
        } else {
            self.emit(I::I32Const(0));
        }
        self.rt(g, "closure");
    }

    fn assign(&mut self, g: &mut Codegen, place: &Place, value: &TExpr) {
        let r = value.repr();
        match place {
            Place::Var(v) => {
                self.expr(g, value);
                let t = self.tmp(r);
                self.emit(I::LocalTee(t));
                self.set_var(g, *v);
                self.emit(I::LocalGet(t));
                self.free(t, r);
            }
            Place::Field(o, sid, idx) => {
                let (off, fr) = g.structs[*sid as usize].offsets[*idx as usize];
                let to = self.tmp(Repr::I32);
                self.expr(g, o);
                self.emit(I::LocalSet(to));
                self.expr(g, value);
                let t = self.tmp(r);
                self.emit(I::LocalSet(t));
                self.emit(I::LocalGet(to));
                self.emit(I::LocalGet(t));
                self.emit(store(off, fr));
                self.emit(I::LocalGet(t));
                self.free(t, r);
                self.free(to, Repr::I32);
            }
            Place::Index(a, i) => {
                let (ta, ti) = (self.local(ValType::I32), self.local(ValType::I32));
                self.expr(g, a);
                self.emit(I::LocalSet(ta));
                self.expr(g, i);
                self.emit(I::I32TruncSatF64S);
                self.emit(I::LocalSet(ti));
                self.expr(g, value);
                let t = self.tmp(r);
                self.emit(I::LocalSet(t));
                self.emit(I::LocalGet(ta));
                self.emit(I::LocalGet(ti));
                self.emit(I::LocalGet(t));
                self.rt(g, if r == Repr::F64 { "arr_set_f64" } else { "arr_set_i32" });
                self.emit(I::LocalGet(t));
                self.free(t, r);
            }
        }
    }

    fn coerce(&mut self, g: &mut Codegen, c: Coercion, a: &TExpr) {
        self.expr(g, a);
        match c {
            Coercion::BoxNum => self.rt(g, "box_f64"),
            Coercion::UnboxNum => self.rt(g, "unbox_f64"),
            Coercion::BoxI32 => {
                self.emit(I::F64ConvertI32S);
                self.rt(g, "box_f64");
            }
            Coercion::UnboxI32 => {
                self.rt(g, "unbox_f64");
                self.emit(I::I32TruncSatF64S);
            }
            Coercion::NumToStr => self.rt(g, "str_from_f64"),
            Coercion::BoolToStr => self.rt(g, "str_from_bool"),
            Coercion::I32ToNum => self.emit(I::F64ConvertI32S),
            Coercion::NumToI32 => self.emit(I::I32TruncSatF64S),
            Coercion::Retag => {}
            Coercion::Discard => {
                if a.repr() != Repr::Void {
                    self.emit(I::Drop);
                }
            }
            Coercion::Truthy => self.truthy(g, &a.ty),
        }
    }

    /// Converts the value on the stack (of type `ty`) to a boolean.
    fn truthy(&mut self, g: &Codegen, ty: &Type) {
        match ty {
            Type::Number => {
                // x !== 0 && x === x (NaN is falsy)
                let t = self.tmp(Repr::F64);
                self.emit(I::LocalTee(t));
                self.emit(I::F64Const(0.0.into()));
                self.emit(I::F64Ne);
                self.emit(I::LocalGet(t));
                self.emit(I::LocalGet(t));
                self.emit(I::F64Eq);
                self.emit(I::I32And);
                self.free(t, Repr::F64);
            }
            Type::String | Type::StrLits(_) => {
                self.rt(g, "str_bytes");
                self.emit(I::I32Const(0));
                self.emit(I::I32Ne);
            }
            Type::Nullable(inner) if **inner == Type::Number || inner.repr() == Repr::I32 => {
                // Boxed: `unbox_f64` always gives back an exact integer for
                // an `i32`-repr inner type, so `x !== 0 && x === x` (the
                // `number` truthiness test) is correct for it too.
                let t = self.tmp(Repr::I32);
                self.emit(I::LocalTee(t));
                self.open(I::If(BlockType::Result(ValType::I32)));
                self.emit(I::LocalGet(t));
                self.rt(g, "unbox_f64");
                self.truthy(g, &Type::Number);
                self.emit(I::Else);
                self.emit(I::I32Const(0));
                self.close();
                self.free(t, Repr::I32);
            }
            Type::Nullable(inner) if inner.is_stringish() => {
                // `as_str(null)` is "", so the length test covers null.
                self.rt(g, "str_bytes");
                self.emit(I::I32Const(0));
                self.emit(I::I32Ne);
            }
            _ => {
                self.emit(I::I32Const(0));
                self.emit(I::I32Ne);
            }
        }
    }

    fn array_hof(&mut self, g: &mut Codegen, kind: ArrayHof, arr: &TExpr, f: &TExpr, arity: usize, result_ty: &Type) {
        let Type::Array(elem) = &arr.ty else { panic!("array method on a non-array") };
        let er = elem.repr();
        let Type::Func(ft) = &f.ty else { panic!("array callback is not a function") };
        let call_ty = g.sig_type(ft);
        let ret_r = ft.ret.repr();
        let (ta, tf, tn, ti) =
            (self.local(ValType::I32), self.local(ValType::I32), self.local(ValType::I32), self.local(ValType::I32));
        let te = self.local(er.val_type().unwrap_or(ValType::I32));
        let rr = match kind {
            ArrayHof::FindIndex => Repr::F64,
            ArrayHof::ForEach => Repr::Void,
            ArrayHof::Some | ArrayHof::Every => Repr::I32,
            _ => Repr::I32,
        };
        let tr = rr.val_type().map(|t| self.local(t));
        self.expr(g, arr);
        self.emit(I::LocalTee(ta));
        self.rt(g, "arr_len");
        self.emit(I::LocalSet(tn));
        self.expr(g, f);
        self.emit(I::LocalSet(tf));
        // The initial result.
        match kind {
            ArrayHof::Map => {
                let Type::Array(out_elem) = result_ty else { panic!("map result is not an array") };
                self.emit(I::I32Const(arr_kind(out_elem.repr())));
                self.emit(I::LocalGet(tn));
                self.rt(g, "arr_new");
            }
            ArrayHof::Filter => {
                self.emit(I::I32Const(arr_kind(er)));
                self.emit(I::I32Const(0));
                self.rt(g, "arr_new");
            }
            ArrayHof::FindIndex => self.emit(I::F64Const((-1.0).into())),
            ArrayHof::Every => self.emit(I::I32Const(1)),
            ArrayHof::Find | ArrayHof::Some => self.emit(I::I32Const(0)),
            ArrayHof::ForEach => {}
        }
        if let Some(tr) = tr {
            self.emit(I::LocalSet(tr));
        }
        self.emit(I::I32Const(0));
        self.emit(I::LocalSet(ti));
        self.open(I::Block(BlockType::Empty));
        let exit = self.depth;
        self.open(I::Loop(BlockType::Empty));
        let top = self.depth;
        self.emit(I::LocalGet(ti));
        self.emit(I::LocalGet(tn));
        self.emit(I::I32GeS);
        self.emit(I::BrIf(self.depth - exit));
        // elem = arr[i]
        self.emit(I::LocalGet(ta));
        self.emit(I::LocalGet(ti));
        self.rt(g, if er == Repr::F64 { "arr_get_f64" } else { "arr_get_i32" });
        self.emit(I::LocalSet(te));
        // r = f(env, elem, i)
        self.emit(I::LocalGet(tf));
        self.emit(I::I32Load(mem(HEADER + 4, Repr::I32)));
        if arity >= 1 {
            self.emit(I::LocalGet(te));
        }
        if arity >= 2 {
            self.emit(I::LocalGet(ti));
            self.emit(I::F64ConvertI32S);
        }
        self.emit(I::LocalGet(tf));
        self.emit(I::I32Load(mem(HEADER, Repr::I32)));
        self.emit(I::CallIndirect { type_index: call_ty, table_index: 0 });
        if g.closures_throw {
            self.check_exc(g);
        }
        let found = |s: &mut Self| {
            s.open(I::If(BlockType::Empty));
        };
        match kind {
            ArrayHof::Map => {
                let tv = self.tmp(ret_r);
                self.emit(I::LocalSet(tv));
                self.emit(I::LocalGet(tr.unwrap()));
                self.emit(I::LocalGet(tv));
                self.rt(g, if ret_r == Repr::F64 { "arr_push_f64" } else { "arr_push_i32" });
                self.emit(I::Drop);
                self.free(tv, ret_r);
            }
            ArrayHof::Filter => {
                found(self);
                self.emit(I::LocalGet(tr.unwrap()));
                self.emit(I::LocalGet(te));
                self.rt(g, if er == Repr::F64 { "arr_push_f64" } else { "arr_push_i32" });
                self.emit(I::Drop);
                self.close();
            }
            ArrayHof::Find => {
                found(self);
                self.emit(I::LocalGet(te));
                if er == Repr::F64 {
                    self.rt(g, "box_f64");
                }
                self.emit(I::LocalSet(tr.unwrap()));
                self.emit(I::Br(self.depth - exit));
                self.close();
            }
            ArrayHof::FindIndex => {
                found(self);
                self.emit(I::LocalGet(ti));
                self.emit(I::F64ConvertI32S);
                self.emit(I::LocalSet(tr.unwrap()));
                self.emit(I::Br(self.depth - exit));
                self.close();
            }
            ArrayHof::Some => {
                found(self);
                self.emit(I::I32Const(1));
                self.emit(I::LocalSet(tr.unwrap()));
                self.emit(I::Br(self.depth - exit));
                self.close();
            }
            ArrayHof::Every => {
                self.emit(I::I32Eqz);
                found(self);
                self.emit(I::I32Const(0));
                self.emit(I::LocalSet(tr.unwrap()));
                self.emit(I::Br(self.depth - exit));
                self.close();
            }
            ArrayHof::ForEach => {
                if ret_r != Repr::Void {
                    self.emit(I::Drop);
                }
            }
        }
        self.emit(I::LocalGet(ti));
        self.emit(I::I32Const(1));
        self.emit(I::I32Add);
        self.emit(I::LocalSet(ti));
        self.emit(I::Br(self.depth - top));
        self.close();
        self.close();
        if let Some(tr) = tr {
            self.emit(I::LocalGet(tr));
        }
    }

    fn array_search(&mut self, g: &mut Codegen, index: bool, eq: EqKind, arr: &TExpr, value: &TExpr) {
        let er = value.repr();
        let (ta, tn, ti) = (self.local(ValType::I32), self.local(ValType::I32), self.local(ValType::I32));
        let tv = self.local(er.val_type().unwrap_or(ValType::I32));
        let tr = self.local(ValType::I32);
        self.expr(g, arr);
        self.emit(I::LocalTee(ta));
        self.rt(g, "arr_len");
        self.emit(I::LocalSet(tn));
        self.expr(g, value);
        self.emit(I::LocalSet(tv));
        self.emit(I::I32Const(-1));
        self.emit(I::LocalSet(tr));
        self.emit(I::I32Const(0));
        self.emit(I::LocalSet(ti));
        self.open(I::Block(BlockType::Empty));
        let exit = self.depth;
        self.open(I::Loop(BlockType::Empty));
        let top = self.depth;
        self.emit(I::LocalGet(ti));
        self.emit(I::LocalGet(tn));
        self.emit(I::I32GeS);
        self.emit(I::BrIf(self.depth - exit));
        self.emit(I::LocalGet(ta));
        self.emit(I::LocalGet(ti));
        self.rt(g, if er == Repr::F64 { "arr_get_f64" } else { "arr_get_i32" });
        self.emit(I::LocalGet(tv));
        self.eq(g, eq);
        self.open(I::If(BlockType::Empty));
        self.emit(I::LocalGet(ti));
        self.emit(I::LocalSet(tr));
        self.emit(I::Br(self.depth - exit));
        self.close();
        self.emit(I::LocalGet(ti));
        self.emit(I::I32Const(1));
        self.emit(I::I32Add);
        self.emit(I::LocalSet(ti));
        self.emit(I::Br(self.depth - top));
        self.close();
        self.close();
        self.emit(I::LocalGet(tr));
        if index {
            self.emit(I::F64ConvertI32S);
        } else {
            self.emit(I::I32Const(-1));
            self.emit(I::I32Ne);
        }
    }
}

fn arr_kind(r: Repr) -> i32 {
    match r {
        Repr::F64 => ARR_F64,
        Repr::Ref => ARR_REF,
        _ => ARR_I32,
    }
}

/// Calls `fs` on every statement and `fe` on every expression in `stmts`,
/// without entering closures (which are other functions).
pub(crate) fn walk_stmts(stmts: &[TStmt], fs: &mut dyn FnMut(&TStmt), fe: &mut dyn FnMut(&TExpr)) {
    for s in stmts {
        walk_stmt(s, fs, fe);
    }
}

pub(crate) fn walk_stmt(s: &TStmt, fs: &mut dyn FnMut(&TStmt), fe: &mut dyn FnMut(&TExpr)) {
    fs(s);
    match s {
        TStmt::Let(_, e) => {
            if let Some(e) = e {
                walk_expr(e, fs, fe);
            }
        }
        TStmt::Expr(e) | TStmt::Throw(e) | TStmt::Trap(e) | TStmt::Return(Some(e)) => walk_expr(e, fs, fe),
        TStmt::If(c, a, b) => {
            walk_expr(c, fs, fe);
            walk_stmts(a, fs, fe);
            walk_stmts(b, fs, fe);
        }
        TStmt::Loop { cond, update, body, .. } => {
            if let Some(c) = cond {
                walk_expr(c, fs, fe);
            }
            if let Some(u) = update {
                walk_expr(u, fs, fe);
            }
            walk_stmts(body, fs, fe);
        }
        TStmt::ForOf { arr, body, .. } => {
            walk_expr(arr, fs, fe);
            walk_stmts(body, fs, fe);
        }
        TStmt::Switch { disc, cases, .. } => {
            walk_expr(disc, fs, fe);
            for (t, b) in cases {
                if let Some(t) = t {
                    walk_expr(t, fs, fe);
                }
                walk_stmts(b, fs, fe);
            }
        }
        TStmt::Try { body, catch, finally } => {
            walk_stmts(body, fs, fe);
            if let Some((_, c)) = catch {
                walk_stmts(c, fs, fe);
            }
            if let Some(f) = finally {
                walk_stmts(f, fs, fe);
            }
        }
        TStmt::Block(b) => walk_stmts(b, fs, fe),
        TStmt::Return(None) | TStmt::Break | TStmt::Continue => {}
    }
}

/// Like `walk_stmts`, with a flag that is true inside the body of a `try`
/// that has a `catch`.
fn walk_covered(stmts: &[TStmt], covered: bool, fs: &mut dyn FnMut(&TStmt, bool), fe: &mut dyn FnMut(&TExpr, bool)) {
    for s in stmts {
        if let TStmt::Try { body, catch: Some((_, c)), finally } = s {
            walk_covered(body, true, fs, fe);
            walk_covered(c, covered, fs, fe);
            if let Some(f) = finally {
                walk_covered(f, covered, fs, fe);
            }
            continue;
        }
        fs(s, covered);
        let (exprs, lists) = stmt_parts(s);
        for e in exprs {
            walk_expr(e, &mut |s| fs(s, covered), &mut |e| fe(e, covered));
        }
        for l in lists {
            walk_covered(l, covered, fs, fe);
        }
    }
}

/// The direct expressions and statement lists of a statement.
pub(crate) fn stmt_parts(s: &TStmt) -> (Vec<&TExpr>, Vec<&[TStmt]>) {
    match s {
        TStmt::Let(_, e) => (e.iter().collect(), Vec::new()),
        TStmt::Expr(e) | TStmt::Throw(e) | TStmt::Trap(e) | TStmt::Return(Some(e)) => (vec![e], Vec::new()),
        TStmt::If(c, a, b) => (vec![c], vec![a, b]),
        TStmt::Loop { cond, update, body, .. } => (cond.iter().chain(update.iter()).collect(), vec![body]),
        TStmt::ForOf { arr, body, .. } => (vec![arr], vec![body]),
        TStmt::Switch { disc, cases, .. } => {
            let mut es = vec![disc];
            es.extend(cases.iter().filter_map(|(t, _)| t.as_ref()));
            (es, cases.iter().map(|(_, b)| b.as_slice()).collect())
        }
        TStmt::Try { body, catch, finally } => {
            let mut ls: Vec<&[TStmt]> = vec![body];
            if let Some((_, c)) = catch {
                ls.push(c);
            }
            if let Some(f) = finally {
                ls.push(f);
            }
            (Vec::new(), ls)
        }
        TStmt::Block(b) => (Vec::new(), vec![b]),
        TStmt::Return(None) | TStmt::Break | TStmt::Continue => (Vec::new(), Vec::new()),
    }
}

pub(crate) fn walk_expr(e: &TExpr, fs: &mut dyn FnMut(&TStmt), fe: &mut dyn FnMut(&TExpr)) {
    fe(e);
    if let TExprKind::Block(stmts, _) = &e.kind {
        walk_stmts(stmts, fs, fe);
    }
    for c in expr_children(e) {
        walk_expr(c, fs, fe);
    }
}

/// The direct sub-expressions of `e`, in evaluation order, for every form
/// (also the ones that only exist before `lower`). A `Block`'s statements
/// are not included (only its value); closures are other functions.
pub(crate) fn expr_children(e: &TExpr) -> Vec<&TExpr> {
    let mut out: Vec<&TExpr> = Vec::new();
    match &e.kind {
        TExprKind::Assign(p, v) => {
            match p {
                Place::Var(_) => {}
                Place::Field(o, ..) => out.push(o),
                Place::Index(a, i) => {
                    out.push(a);
                    out.push(i);
                }
            }
            out.push(v);
        }
        TExprKind::Field(o, ..)
        | TExprKind::Neg(o)
        | TExprKind::Not(o)
        | TExprKind::IsNull(o)
        | TExprKind::Coerce(_, o)
        | TExprKind::UnionTag(o)
        | TExprKind::UnionIs(o, _)
        | TExprKind::InstanceOf(o, _)
        | TExprKind::Await(o)
        | TExprKind::SignalNew(o)
        | TExprKind::SignalGet(o)
        | TExprKind::SignalPeek(o)
        | TExprKind::ComputedNew(o)
        | TExprKind::ComputedGet(o)
        | TExprKind::EffectNew(o)
        | TExprKind::FrameNew(o) => out.push(o),
        TExprKind::Index(a, b)
        | TExprKind::Num2(_, a, b)
        | TExprKind::Int2(_, a, b)
        | TExprKind::Cmp(_, _, a, b)
        | TExprKind::StrCmp(_, a, b)
        | TExprKind::Concat(a, b)
        | TExprKind::And(a, b)
        | TExprKind::Or(a, b)
        | TExprKind::SignalSet(a, b)
        | TExprKind::ArrayHof { arr: a, f: b, .. }
        | TExprKind::ArraySearch { arr: a, value: b, .. }
        | TExprKind::TimerNew(a, _, b)
        | TExprKind::DialogCall(_, a, b) => {
            out.push(a);
            out.push(b);
        }
        TExprKind::Cond(a, b, c) | TExprKind::FilesCall(_, a, b, c) => out.extend([&**a, &**b, &**c]),
        TExprKind::NetFetchCall(a, b, c, d, f) => out.extend([&**a, &**b, &**c, &**d, &**f]),
        TExprKind::Call(_, args) | TExprKind::Rt(_, args) | TExprKind::MathOp(_, args) | TExprKind::StructLit(_, args) => {
            out.extend(args.iter())
        }
        TExprKind::CallClosure(c, args) | TExprKind::MethodCall(_, _, c, args) => {
            out.push(c);
            out.extend(args.iter());
        }
        TExprKind::ArrayLit(items) => out.extend(items.iter().map(|(_, a)| a)),
        TExprKind::Block(_, v) => out.push(v),
        TExprKind::Jsx(j) => jsx_exprs(j, &mut out),
        TExprKind::Num(_)
        | TExprKind::Bool(_)
        | TExprKind::Str(_)
        | TExprKind::Null
        | TExprKind::Var(_)
        | TExprKind::Closure(_)
        | TExprKind::ThunkOf(_)
        | TExprKind::Navigate(_)
        | TExprKind::NavigatePush(_)
        | TExprKind::NavigateBack => {}
    }
    out
}

/// Like `expr_children`, for changing them. JSX gives none.
pub(crate) fn expr_children_mut(e: &mut TExpr) -> Vec<&mut TExpr> {
    let mut out: Vec<&mut TExpr> = Vec::new();
    match &mut e.kind {
        TExprKind::Assign(p, v) => {
            match p {
                Place::Var(_) => {}
                Place::Field(o, ..) => out.push(o),
                Place::Index(a, i) => {
                    out.push(a);
                    out.push(i);
                }
            }
            out.push(v);
        }
        TExprKind::Field(o, ..)
        | TExprKind::Neg(o)
        | TExprKind::Not(o)
        | TExprKind::IsNull(o)
        | TExprKind::Coerce(_, o)
        | TExprKind::UnionTag(o)
        | TExprKind::UnionIs(o, _)
        | TExprKind::InstanceOf(o, _)
        | TExprKind::Await(o)
        | TExprKind::SignalNew(o)
        | TExprKind::SignalGet(o)
        | TExprKind::SignalPeek(o)
        | TExprKind::ComputedNew(o)
        | TExprKind::ComputedGet(o)
        | TExprKind::EffectNew(o)
        | TExprKind::FrameNew(o) => out.push(o),
        TExprKind::Index(a, b)
        | TExprKind::Num2(_, a, b)
        | TExprKind::Int2(_, a, b)
        | TExprKind::Cmp(_, _, a, b)
        | TExprKind::StrCmp(_, a, b)
        | TExprKind::Concat(a, b)
        | TExprKind::And(a, b)
        | TExprKind::Or(a, b)
        | TExprKind::SignalSet(a, b)
        | TExprKind::ArrayHof { arr: a, f: b, .. }
        | TExprKind::ArraySearch { arr: a, value: b, .. }
        | TExprKind::TimerNew(a, _, b)
        | TExprKind::DialogCall(_, a, b) => {
            out.push(a);
            out.push(b);
        }
        TExprKind::Cond(a, b, c) | TExprKind::FilesCall(_, a, b, c) => out.extend([&mut **a, &mut **b, &mut **c]),
        TExprKind::NetFetchCall(a, b, c, d, f) => out.extend([&mut **a, &mut **b, &mut **c, &mut **d, &mut **f]),
        TExprKind::Call(_, args) | TExprKind::Rt(_, args) | TExprKind::MathOp(_, args) | TExprKind::StructLit(_, args) => {
            out.extend(args.iter_mut())
        }
        TExprKind::CallClosure(c, args) | TExprKind::MethodCall(_, _, c, args) => {
            out.push(c);
            out.extend(args.iter_mut());
        }
        TExprKind::ArrayLit(items) => out.extend(items.iter_mut().map(|(_, a)| a)),
        TExprKind::Block(_, v) => out.push(v),
        TExprKind::Jsx(_) => {}
        TExprKind::Num(_)
        | TExprKind::Bool(_)
        | TExprKind::Str(_)
        | TExprKind::Null
        | TExprKind::Var(_)
        | TExprKind::Closure(_)
        | TExprKind::ThunkOf(_)
        | TExprKind::Navigate(_)
        | TExprKind::NavigatePush(_)
        | TExprKind::NavigateBack => {}
    }
    out
}

fn jsx_exprs<'a>(j: &'a TJsx, out: &mut Vec<&'a TExpr>) {
    match j {
        TJsx::Control { props, children, .. } => {
            out.extend(props.iter().map(|p| &p.value));
            match children {
                TChildren::None => {}
                TChildren::Text(parts) => out.extend(parts.iter()),
                TChildren::Nodes(nodes) => {
                    for n in nodes {
                        match n {
                            TChild::Element(j) => jsx_exprs(j, out),
                            TChild::Expr(e) => out.push(e),
                        }
                    }
                }
            }
        }
        TJsx::Component { props, .. } => out.extend(props.iter()),
    }
}
