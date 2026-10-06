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
    type_id: u32,
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
    };
    g.find_captures();
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
    let entry_body = g.entry(main_idx);
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

    /// The GC type id a union member has at runtime, to narrow via the
    /// header's `type_id` (`UnionIs`).
    fn type_id_of(&self, member: &Type) -> u32 {
        match member {
            Type::Struct(sid) => self.structs[*sid as usize].type_id,
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
                (self.structs[d as usize].type_id, idx)
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
            let type_id = self.user_type(size, refs);
            self.structs.push(StructLayout { type_id, offsets: offsets.into_iter().zip(reprs).collect() });
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

    fn entry(&mut self, main_idx: u32) -> Function {
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
        f.instruction(&I::End);
        f
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
        for r in &sig.params {
            f.instruction(&I::Call(if *r == Repr::F64 { rt("arg_f64") } else { rt("arg_i32") }));
        }
        f.instruction(&I::LocalGet(0));
        f.instruction(&I::I32Load(mem(HEADER, Repr::I32)));
        f.instruction(&I::CallIndirect { type_index: call_ty, table_index: 0 });
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
        TStmt::Expr(e) | TStmt::Return(Some(e)) | TStmt::Throw(e) => expr_vars(e, out),
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
        }
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
                if let Some(e) = e {
                    self.expr(g, e);
                }
                self.emit(I::Return);
            }
            TStmt::Break => {
                let l = self.labels.last().expect("break inside a loop or switch").brk;
                self.emit(I::Br(self.depth - l));
            }
            TStmt::Continue => {
                let l = self.labels.iter().rev().find_map(|l| l.cont).expect("continue inside a loop");
                self.emit(I::Br(self.depth - l));
            }
            TStmt::Throw(e) => {
                self.expr(g, e);
                self.rt(g, "throw");
                self.emit(I::Unreachable);
            }
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
                let layout_type = g.structs[*sid as usize].type_id;
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
            }
            TExprKind::InstanceOf(o, sid) => {
                let ids: Vec<u32> = g.class_descendants(*sid).iter().map(|s| g.structs[*s as usize].type_id).collect();
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
