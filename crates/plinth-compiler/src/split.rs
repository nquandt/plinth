//! App modules: a `.plnt` holds only the app code (SPEC.md §10.1).
//!
//! The compiler writes `app.wasm` as a standalone core module. It imports
//! the runtime functions, the memory and the function table from the module
//! `plinth-rt`, and it holds the app's own types, functions, globals, table
//! entries and passive data. It does not hold any runtime code.
//!
//! The host supplies the runtime. At load time, `load_app` checks the app
//! module and remaps its indices into the index spaces of the runtime, and
//! the append linker (`link::link`) makes one module from the two.
//!
//! An app module does not depend on one runtime build. It names the core
//! version that it needs in the custom section `plinth-core`, it imports
//! runtime functions by name and type, and its table indices are relative to
//! the imported global `table_base`. Thus a `.plnt` is built one time and
//! runs on every host that has a core of that major version (`cores`).

use crate::link::{AppCode, Layout};
use crate::rt_abi;
use anyhow::{Context as _, Result, anyhow, bail, ensure};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use wasm_encoder::reencode::{Reencode, RoundtripReencoder};
use wasm_encoder::{
    CodeSection, ConstExpr, CustomSection, DataCountSection, DataSection, ElementSection, Elements, EntityType,
    FunctionSection, GlobalSection, GlobalType, ImportSection, MemoryType, Module, RefType, StartSection, TableType,
    TypeSection, ValType,
};
use wasmparser::{BinaryReader, Parser, Payload, TypeRef};

/// The import module name of the runtime.
pub const RT_MODULE: &str = "plinth-rt";
/// The custom section that names the core version of an app module. It
/// has the same name as the section that holds a core's own version.
pub const CORE_SECTION: &str = crate::cores::CORE_SECTION;
/// The import name of the global that holds the app's first table index.
pub const TABLE_BASE: &str = "table_base";

/// The core version that this compiler writes, `MAJOR.MINOR`.
pub fn core_needed() -> String {
    format!("{}.{}", rt_abi::CORE_MAJOR, rt_abi::CORE_MINOR)
}

/// The value of the manifest field `runtime`: `plinth-core/MAJOR.MINOR`.
pub fn runtime_field() -> String {
    format!("plinth-core/{}", core_needed())
}

/// The id of a runtime build: `plinth-rt/` and the first 16 hex digits of
/// the SHA-256 of the runtime module. Tools show it; packages do not use it.
pub fn runtime_build_id(rt: &[u8]) -> String {
    let digest = Sha256::digest(rt);
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("plinth-rt/{hex}")
}

/// The layout that codegen uses for a standalone app module: the runtime
/// functions are imports, in the order of `rt_abi::FUNCTIONS`, global 0 is
/// the imported `table_base`, and the app's own index spaces start at zero.
/// Table indices are relative to `table_base`.
pub struct AppLayout {
    pub layout: Layout,
    import_types: Vec<(Vec<ValType>, Vec<ValType>)>,
    /// `(name without the prefix, type index)` for each import.
    imports: Vec<(&'static str, u32)>,
}

/// `extra` adds functions after `rt_abi::FUNCTIONS` (the dev functions of
/// `plinth dev`).
pub fn app_layout(extra: &[(&'static str, &'static [ValType], &'static [ValType])]) -> AppLayout {
    let mut import_types: Vec<(Vec<ValType>, Vec<ValType>)> = Vec::new();
    let mut imports = Vec::new();
    let mut rt_funcs = HashMap::new();
    for (i, &(name, params, results)) in rt_abi::FUNCTIONS.iter().chain(extra.iter()).enumerate() {
        let key = (params.to_vec(), results.to_vec());
        let ty = match import_types.iter().position(|t| *t == key) {
            Some(t) => t as u32,
            None => {
                import_types.push(key);
                import_types.len() as u32 - 1
            }
        };
        imports.push((name, ty));
        rt_funcs.insert(name, i as u32);
    }
    let layout = Layout {
        type_count: import_types.len() as u32,
        func_count: imports.len() as u32,
        global_count: 1,
        table_size: 0,
        data_count: 0,
        rt_start: None,
        rt_funcs,
        table_base_global: Some(0),
        exports: HashMap::new(),
        missing: Vec::new(),
    };
    AppLayout { layout, import_types, imports }
}

/// Maps old indices to new ones while the code is copied.
struct Remap {
    funcs: Vec<u32>,
    types: u32,
    globals: u32,
    data: u32,
    /// At load time: the value of the imported global 0 (`table_base`).
    /// `global.get 0` becomes this constant, and the app's own globals move
    /// down by one.
    table_base: Option<u32>,
}

impl Reencode for Remap {
    type Error = std::convert::Infallible;

    fn instruction<'a>(
        &mut self,
        op: wasmparser::Operator<'a>,
    ) -> Result<wasm_encoder::Instruction<'a>, wasm_encoder::reencode::Error<Self::Error>> {
        match (self.table_base, &op) {
            (Some(base), wasmparser::Operator::GlobalGet { global_index: 0 }) => {
                Ok(wasm_encoder::Instruction::I32Const(base as i32))
            }
            _ => wasm_encoder::reencode::utils::instruction(self, op),
        }
    }

    fn function_index(&mut self, func: u32) -> Result<u32, wasm_encoder::reencode::Error<Self::Error>> {
        Ok(self.funcs[func as usize])
    }
    fn type_index(&mut self, ty: u32) -> Result<u32, wasm_encoder::reencode::Error<Self::Error>> {
        Ok(self.types + ty)
    }
    fn global_index(&mut self, global: u32) -> Result<u32, wasm_encoder::reencode::Error<Self::Error>> {
        // At load time the imported `table_base` (global 0) is gone.
        Ok(self.globals + global - u32::from(self.table_base.is_some()))
    }
    fn data_index(&mut self, data: u32) -> Result<u32, wasm_encoder::reencode::Error<Self::Error>> {
        Ok(self.data + data)
    }
}

/// Copies one encoded function body (with its size prefix) through `remap`.
fn remap_body(remap: &mut Remap, encoded: &[u8]) -> Result<wasm_encoder::Function> {
    let mut r = BinaryReader::new(encoded, 0);
    let size = r.read_var_u32()? as usize;
    let start = r.original_position();
    let body = wasmparser::FunctionBody::new(BinaryReader::new(&encoded[start..start + size], 0));
    let mut f = remap.new_function_with_parsed_locals(&body).map_err(|e| anyhow!("{e:?}"))?;
    let mut ops = body.get_operators_reader()?;
    while !ops.eof() {
        f.instruction(&remap.parse_instruction(&mut ops).map_err(|e| anyhow!("{e:?}"))?);
    }
    Ok(f)
}

fn encoded(f: &wasm_encoder::Function) -> Vec<u8> {
    let mut bytes = Vec::new();
    wasm_encoder::Encode::encode(f, &mut bytes);
    bytes
}

/// Encodes the app code as a standalone app module. It imports only the
/// runtime functions that the app calls.
pub fn encode_app(al: &AppLayout, app: &AppCode) -> Result<Vec<u8>> {
    let n = al.imports.len() as u32;
    let bodies: Vec<Vec<u8>> = app.funcs.iter().map(|(_, f)| encoded(f)).collect();

    // The imports that the app uses.
    let mut used = vec![false; n as usize];
    for body in &bodies {
        for f in crate::link::callees_of(body)? {
            if f < n {
                used[f as usize] = true;
            }
        }
    }
    for &f in &app.table {
        if f < n {
            used[f as usize] = true;
        }
    }
    let mut funcs = vec![0u32; n as usize + app.funcs.len()];
    let mut kept = 0u32;
    for (i, u) in used.iter().enumerate() {
        if *u {
            funcs[i] = kept;
            kept += 1;
        }
    }
    for j in 0..app.funcs.len() {
        funcs[n as usize + j] = kept + j as u32;
    }
    let mut remap = Remap { funcs, types: 0, globals: 0, data: 0, table_base: None };

    let mut types = TypeSection::new();
    for (params, results) in al.import_types.iter().chain(app.types.iter()) {
        types.ty().function(params.iter().copied(), results.iter().copied());
    }
    let mut imports = ImportSection::new();
    for (i, &(name, ty)) in al.imports.iter().enumerate() {
        if used[i] {
            imports.import(RT_MODULE, name, EntityType::Function(ty));
        }
    }
    imports.import(
        RT_MODULE,
        "memory",
        EntityType::Memory(MemoryType { minimum: 0, maximum: None, memory64: false, shared: false, page_size_log2: None }),
    );
    imports.import(
        RT_MODULE,
        TABLE_BASE,
        EntityType::Global(GlobalType { val_type: ValType::I32, mutable: false, shared: false }),
    );
    let table_min = app.table.len() as u64;
    imports.import(
        RT_MODULE,
        "table",
        EntityType::Table(TableType { element_type: RefType::FUNCREF, table64: false, minimum: table_min, maximum: None, shared: false }),
    );
    let mut functions = FunctionSection::new();
    let mut code = CodeSection::new();
    for ((ty, _), body) in app.funcs.iter().zip(&bodies) {
        functions.function(*ty);
        code.function(&remap_body(&mut remap, body)?);
    }
    let mut globals = GlobalSection::new();
    for (ty, init) in &app.globals {
        globals.global(GlobalType { val_type: *ty, mutable: true, shared: false }, init);
    }
    let mut elements = ElementSection::new();
    let table: Vec<u32> = app.table.iter().map(|&f| remap.funcs[f as usize]).collect();
    if !table.is_empty() {
        elements.active(
            Some(0),
            &ConstExpr::global_get(0),
            Elements::Functions(std::borrow::Cow::Borrowed(&table)),
        );
    }
    let mut data = DataSection::new();
    for seg in &app.data {
        data.passive(seg.iter().copied());
    }

    let mut module = Module::new();
    module.section(&types).section(&imports).section(&functions);
    if !app.globals.is_empty() {
        module.section(&globals);
    }
    if let Some(start) = app.start {
        module.section(&StartSection { function_index: remap.funcs[start as usize] });
    }
    if !table.is_empty() {
        module.section(&elements);
    }
    module.section(&DataCountSection { count: app.data.len() as u32 });
    module.section(&code).section(&data);
    module.section(&CustomSection { name: CORE_SECTION.into(), data: core_needed().as_bytes().into() });
    let bytes = module.finish();
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&bytes)
        .context("the app module is not valid")?;
    Ok(bytes)
}

/// Returns the core version that an app module needs, if it names one.
pub fn app_core_version(app: &[u8]) -> Option<crate::cores::Version> {
    crate::cores::core_version(app)
}

/// True when `bytes` is a core module (an app module), not a component.
pub fn is_app_module(bytes: &[u8]) -> bool {
    bytes.len() >= 8 && bytes[..8] == [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]
}

/// Checks an app module against the runtime `rt` and remaps it into the
/// runtime's index spaces, for `link::link`.
///
/// The app must need the major version of the core `rt`, and a minor
/// version that is not higher. It may import only functions that the core
/// exports (same name and type), `table_base`, and the memory and the table
/// of `plinth-rt`. It must not define a memory or a table, export anything,
/// or have active data. Its one element segment must start at `table_base`.
pub fn load_app(rt: &[u8], rt_layout: &Layout, app: &[u8]) -> Result<AppCode> {
    ensure!(is_app_module(app), "app.wasm is not a core Wasm module");
    let need = app_core_version(app).with_context(|| format!("app.wasm does not name its core (no `{CORE_SECTION}` section)"))?;
    let have = crate::cores::core_version(rt).context("the runtime has no core version")?;
    ensure!(
        need.0 == have.0 && need.1 <= have.1,
        "the app needs core {}.{}, but this core is {}.{}",
        need.0,
        need.1,
        have.0,
        have.1
    );
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(app)
        .context("app.wasm is not valid")?;

    let mut out = AppCode::default();
    let mut funcs: Vec<u32> = Vec::new();
    let mut types: Vec<wasmparser::FuncType> = Vec::new();
    let mut bodies: Vec<Vec<u8>> = Vec::new();
    let mut func_types: Vec<u32> = Vec::new();
    let mut raw_globals: Vec<(wasmparser::GlobalType, Vec<u8>)> = Vec::new();
    let mut table: Vec<u32> = Vec::new();
    let mut start = None;
    for payload in Parser::new(0).parse_all(app) {
        match payload? {
            Payload::Version { .. } | Payload::End(_) | Payload::DataCountSection { .. } | Payload::CodeSectionStart { .. } => {}
            Payload::CustomSection(_) => {}
            Payload::TypeSection(r) => {
                for group in r {
                    for ty in group?.into_types() {
                        match ty.composite_type.inner {
                            wasmparser::CompositeInnerType::Func(f) => types.push(f),
                            _ => bail!("app.wasm has a non-function type"),
                        }
                    }
                }
            }
            Payload::ImportSection(r) => {
                for imp in r.into_imports() {
                    let imp = imp?;
                    ensure!(imp.module == RT_MODULE, "app.wasm imports from `{}`; only `{RT_MODULE}` is allowed", imp.module);
                    match imp.ty {
                        TypeRef::Func(t) => {
                            let (idx, params, results) = rt_layout.exports.get(imp.name).with_context(|| {
                                format!("app.wasm imports `{}`, which this core does not have; a newer core may run it", imp.name)
                            })?;
                            let ty = types.get(t as usize).context("bad import type")?;
                            let same = ty.params().iter().map(val_type).eq(params.iter().copied())
                                && ty.results().iter().map(val_type).eq(results.iter().copied());
                            ensure!(same, "app.wasm imports `{}` with a wrong type", imp.name);
                            funcs.push(*idx);
                        }
                        TypeRef::Global(g) if imp.name == TABLE_BASE && !g.mutable && g.content_type == wasmparser::ValType::I32 => {}
                        TypeRef::Memory(_) if imp.name == "memory" => {}
                        TypeRef::Table(_) if imp.name == "table" => {}
                        _ => bail!("app.wasm has a bad import `{}`", imp.name),
                    }
                }
            }
            Payload::FunctionSection(r) => {
                for t in r {
                    func_types.push(t?);
                }
            }
            Payload::GlobalSection(r) => {
                for g in r {
                    let g = g?;
                    let mut ops = g.init_expr.get_operators_reader();
                    let mut init = Vec::new();
                    while !ops.eof() {
                        match ops.read()? {
                            wasmparser::Operator::I32Const { value } => init.push(value),
                            wasmparser::Operator::End => {}
                            _ => bail!("app.wasm has a global with an unsupported initializer"),
                        }
                    }
                    ensure!(init.len() == 1, "app.wasm has a global with an unsupported initializer");
                    raw_globals.push((g.ty, init[0].to_le_bytes().to_vec()));
                }
            }
            Payload::StartSection { func, .. } => start = Some(func),
            Payload::ElementSection(r) => {
                for el in r {
                    let el = el?;
                    let wasmparser::ElementKind::Active { table_index, offset_expr } = el.kind else {
                        bail!("app.wasm has a passive or declared element segment");
                    };
                    ensure!(table_index.unwrap_or(0) == 0 && table.is_empty(), "app.wasm has more than one element segment");
                    let mut ops = offset_expr.get_operators_reader();
                    ensure!(
                        matches!(ops.read()?, wasmparser::Operator::GlobalGet { global_index: 0 }),
                        "the element segment of app.wasm must start at `table_base`"
                    );
                    let wasmparser::ElementItems::Functions(fs) = el.items else {
                        bail!("app.wasm has an element segment of expressions");
                    };
                    for f in fs {
                        table.push(f?);
                    }
                }
            }
            Payload::DataSection(r) => {
                for d in r {
                    let d = d?;
                    ensure!(matches!(d.kind, wasmparser::DataKind::Passive), "app.wasm has an active data segment");
                    out.data.push(d.data.to_vec());
                }
            }
            Payload::CodeSectionEntry(body) => {
                let range = body.range();
                let mut bytes = Vec::new();
                ((range.end - range.start) as u32).encode_leb(&mut bytes);
                bytes.extend_from_slice(&app[range]);
                bodies.push(bytes);
            }
            Payload::MemorySection(_) => bail!("app.wasm must not define a memory"),
            Payload::TableSection(_) => bail!("app.wasm must not define a table"),
            Payload::ExportSection(r) => ensure!(r.count() == 0, "app.wasm must not export anything"),
            other => bail!("app.wasm has an unsupported section ({:?})", other.as_section().map(|(id, _)| id)),
        }
    }

    let defined_base = rt_layout.func_count;
    for j in 0..func_types.len() as u32 {
        funcs.push(defined_base + j);
    }
    let mut remap = Remap {
        funcs,
        types: rt_layout.type_count,
        globals: rt_layout.global_count,
        data: rt_layout.data_count,
        table_base: Some(rt_layout.table_size),
    };
    for ty in &types {
        out.types.push((ty.params().iter().map(val_type).collect(), ty.results().iter().map(val_type).collect()));
    }
    for (ty, body) in func_types.iter().zip(&bodies) {
        out.funcs.push((rt_layout.type_count + ty, remap_body(&mut remap, body)?));
    }
    for (ty, init) in raw_globals {
        let value = i32::from_le_bytes(init.try_into().unwrap());
        let val_type = RoundtripReencoder.val_type(ty.content_type).map_err(|e| anyhow!("{e:?}"))?;
        out.globals.push((val_type, ConstExpr::i32_const(value)));
    }
    out.table = table.iter().map(|&f| remap.funcs[f as usize]).collect();
    out.start = start.map(|f| remap.funcs[f as usize]);
    Ok(out)
}

trait EncodeLeb {
    fn encode_leb(self, out: &mut Vec<u8>);
}

impl EncodeLeb for u32 {
    fn encode_leb(self, out: &mut Vec<u8>) {
        wasm_encoder::Encode::encode(&self, out);
    }
}

fn val_type(t: &wasmparser::ValType) -> ValType {
    match t {
        wasmparser::ValType::I32 => ValType::I32,
        wasmparser::ValType::I64 => ValType::I64,
        wasmparser::ValType::F32 => ValType::F32,
        wasmparser::ValType::F64 => ValType::F64,
        wasmparser::ValType::V128 => ValType::V128,
        wasmparser::ValType::Ref(_) => ValType::FUNCREF,
    }
}

/// Links an app module into the runtime `rt` and wraps the result as a
/// `plinth:app` component, ready to run.
pub fn link_app(rt: &[u8], app: &[u8]) -> Result<Vec<u8>> {
    let layout = crate::link::layout(rt)?;
    let code = load_app(rt, &layout, app)?;
    let core = crate::link::link(rt, &layout, &code)?;
    crate::link::componentize(&core)
}
