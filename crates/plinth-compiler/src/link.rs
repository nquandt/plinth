//! The append linker (SPEC.md §5.1 step 8).
//!
//! The linker adds the generated app code to the `plinth-rt` core module. It
//! only appends: new types, functions, globals, table entries and passive
//! data segments go after the runtime's own. Thus no index inside the
//! runtime changes, and the runtime's code bodies are copied as raw bytes.
//!
//! The linker also:
//! - grows the function table for the app's closures,
//! - sets the start function (it gives the app entry point to the runtime),
//! - removes the `__plinth_rt_*` exports, which only the app code uses,
//! - stubs the runtime functions that the app cannot reach (`stub_unused`).
//!   A stub keeps its index, so the append rule still holds.

use crate::rt_abi;
use anyhow::{Context as _, Result, bail};
use std::collections::HashMap;
use std::ops::Range;
use wasm_encoder::{
    CodeSection, ConstExpr, ElementSection, Elements, Encode, ExportKind, ExportSection, FunctionSection,
    GlobalSection, GlobalType, Module, RawSection, RefType, TableSection, TableType, TypeSection, ValType,
};
use wasmparser::{BinaryReader, Parser, Payload, TypeRef};

/// The index spaces of the runtime module. Generated code uses them to
/// compute absolute indices.
#[derive(Debug, Clone)]
pub struct Layout {
    pub type_count: u32,
    pub func_count: u32,
    pub global_count: u32,
    pub table_size: u32,
    pub data_count: u32,
    pub rt_start: Option<u32>,
    /// Runtime function name (without the prefix) → function index.
    pub rt_funcs: HashMap<&'static str, u32>,
    /// For an app module (SPEC.md §10.4): the imported global that holds the
    /// first table index of the app. Table indices are then relative to it.
    pub table_base_global: Option<u32>,
    /// Every `__plinth_rt_*` export of the core: name without the prefix →
    /// `(function index, params, results)`. An app links against these, so
    /// an older core can run apps that use only what it has.
    pub exports: HashMap<String, (u32, Vec<ValType>, Vec<ValType>)>,
    /// The `rt_abi` functions that this core lacks or has with another
    /// type. The compiler needs none missing in its own core.
    pub missing: Vec<&'static str>,
}

impl Layout {
    pub fn rt(&self, name: &str) -> u32 {
        *self.rt_funcs.get(name).unwrap_or_else(|| panic!("unknown runtime function `{name}`"))
    }
}

/// The generated code, with indices that are absolute in the linked module.
#[derive(Default)]
pub struct AppCode {
    pub types: Vec<(Vec<ValType>, Vec<ValType>)>,
    /// `(type index, body)` in function index order, from `Layout::func_count`.
    pub funcs: Vec<(u32, wasm_encoder::Function)>,
    pub globals: Vec<(ValType, ConstExpr)>,
    /// Function indices that go into the table, from `Layout::table_size`.
    pub table: Vec<u32>,
    /// Passive data segments, from `Layout::data_count`.
    pub data: Vec<Vec<u8>>,
    pub start: Option<u32>,
}

struct RtSections<'a> {
    bytes: &'a [u8],
    /// `(section id, content range)` in file order. Custom sections have id 0.
    sections: Vec<(u8, Range<usize>)>,
}

fn sections(bytes: &[u8]) -> Result<RtSections<'_>> {
    let mut sections = Vec::new();
    for payload in Parser::new(0).parse_all(bytes) {
        let payload = payload?;
        if let Payload::CodeSectionEntry(_) = payload {
            continue;
        }
        if let Some((id, range)) = payload.as_section() {
            sections.push((id, range));
        }
    }
    Ok(RtSections { bytes, sections })
}

/// Reads the index spaces and the runtime function indices.
pub fn layout(rt: &[u8]) -> Result<Layout> {
    let mut types: Vec<wasmparser::FuncType> = Vec::new();
    let mut func_types: Vec<u32> = Vec::new();
    let (mut global_count, mut table_size, mut data_count, mut rt_start) = (0, 0, 0, None);
    let mut exports: HashMap<String, u32> = HashMap::new();
    let mut tables = 0;
    for payload in Parser::new(0).parse_all(rt) {
        match payload? {
            Payload::TypeSection(r) => {
                for group in r {
                    for ty in group?.into_types() {
                        match ty.composite_type.inner {
                            wasmparser::CompositeInnerType::Func(f) => types.push(f),
                            _ => bail!("plinth-rt has a non-function type"),
                        }
                    }
                }
            }
            Payload::ImportSection(r) => {
                for imp in r.into_imports() {
                    match imp?.ty {
                        TypeRef::Func(t) => func_types.push(t),
                        TypeRef::Global(_) => global_count += 1,
                        TypeRef::Table(_) => bail!("plinth-rt must not import a table"),
                        _ => {}
                    }
                }
            }
            Payload::FunctionSection(r) => {
                for t in r {
                    func_types.push(t?);
                }
            }
            Payload::TableSection(r) => {
                for t in r {
                    let t = t?;
                    tables += 1;
                    table_size = t.ty.initial as u32;
                    if t.ty.maximum.is_some() {
                        bail!("the plinth-rt table has a maximum size; link it with --growable-table");
                    }
                }
            }
            Payload::GlobalSection(r) => global_count += r.count(),
            Payload::ExportSection(r) => {
                for e in r {
                    let e = e?;
                    if e.kind == wasmparser::ExternalKind::Func {
                        exports.insert(e.name.to_owned(), e.index);
                    }
                }
            }
            Payload::StartSection { func, .. } => rt_start = Some(func),
            Payload::DataSection(r) => data_count = r.count(),
            _ => {}
        }
    }
    if tables != 1 {
        bail!("plinth-rt must have exactly one table, found {tables}");
    }
    let mut abi_exports: HashMap<String, (u32, Vec<ValType>, Vec<ValType>)> = HashMap::new();
    for (full, &idx) in &exports {
        let Some(name) = full.strip_prefix(rt_abi::PREFIX) else { continue };
        let ty = &types[func_types[idx as usize] as usize];
        let params: Vec<ValType> = ty.params().iter().map(val_type).collect();
        let results: Vec<ValType> = ty.results().iter().map(val_type).collect();
        abi_exports.insert(name.to_owned(), (idx, params, results));
    }
    let mut rt_funcs = HashMap::new();
    let mut missing = Vec::new();
    for &(name, params, results) in rt_abi::FUNCTIONS {
        match abi_exports.get(name) {
            Some((idx, p, r)) if p.as_slice() == params && r.as_slice() == results => {
                rt_funcs.insert(name, *idx);
            }
            _ => missing.push(name),
        }
    }
    Ok(Layout {
        type_count: types.len() as u32,
        func_count: func_types.len() as u32,
        global_count,
        table_size,
        data_count,
        rt_start,
        rt_funcs,
        table_base_global: None,
        exports: abi_exports,
        missing,
    })
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

/// Splits the content of a vector section into its count and the raw bytes
/// of its entries.
fn split_vec(content: &[u8]) -> Result<(u32, &[u8])> {
    let mut r = BinaryReader::new(content, 0);
    let count = r.read_var_u32()?;
    Ok((count, &content[r.original_position()..]))
}

/// The raw entries of an encoded section (without its count).
fn entries_of(section: &impl Encode) -> Vec<u8> {
    let mut bytes = Vec::new();
    section.encode(&mut bytes);
    // Skip the size prefix and the count.
    let mut r = BinaryReader::new(&bytes, 0);
    let _size = r.read_var_u32().unwrap();
    let _count = r.read_var_u32().unwrap();
    bytes[r.original_position()..].to_vec()
}

fn append_section(id: u8, old: Option<&[u8]>, new_count: u32, new_entries: &[u8]) -> Result<RawSection<'static>> {
    let (old_count, old_entries) = match old {
        Some(content) => split_vec(content)?,
        None => (0, &[][..]),
    };
    let mut data = Vec::new();
    (old_count + new_count).encode(&mut data);
    data.extend_from_slice(old_entries);
    data.extend_from_slice(new_entries);
    Ok(RawSection { id, data: Box::leak(data.into_boxed_slice()) })
}

/// The order of the standard sections in a module.
const ORDER: [u8; 13] = [1, 2, 3, 4, 5, 13, 6, 7, 8, 9, 12, 10, 11];

/// Links the app code into the runtime module. `layout` must come from the
/// same `rt` bytes.
pub fn link(rt: &[u8], layout: &Layout, app: &AppCode) -> Result<Vec<u8>> {
    let rt_sections = sections(rt)?;
    let content = |id: u8| -> Option<&[u8]> {
        rt_sections.sections.iter().find(|(i, _)| *i == id).map(|(_, r)| &rt_sections.bytes[r.clone()])
    };

    // New entries for each changed section.
    let mut types = TypeSection::new();
    for (params, results) in &app.types {
        types.ty().function(params.iter().copied(), results.iter().copied());
    }
    let mut funcs = FunctionSection::new();
    let mut code = CodeSection::new();
    for (ty, body) in &app.funcs {
        funcs.function(*ty);
        code.function(body);
    }
    let mut globals = GlobalSection::new();
    for (ty, init) in &app.globals {
        globals.global(GlobalType { val_type: *ty, mutable: true, shared: false }, init);
    }
    let mut elements = ElementSection::new();
    if !app.table.is_empty() {
        elements.active(
            None,
            &ConstExpr::i32_const(layout.table_size as i32),
            Elements::Functions(std::borrow::Cow::Borrowed(&app.table)),
        );
    }
    let mut data = wasm_encoder::DataSection::new();
    for seg in &app.data {
        data.passive(seg.iter().copied());
    }

    let mut out = Module::new();
    let mut emitted: Vec<u8> = Vec::new();
    let emit_standard = |out: &mut Module, id: u8| -> Result<()> {
        let old = content(id);
        match id {
            1 => out.section(&append_section(1, old, types.len(), &entries_of(&types))?),
            3 => out.section(&append_section(3, old, funcs.len(), &entries_of(&funcs))?),
            10 => {
                let app_code = entries_of(&code);
                let rt_code = match old {
                    Some(old) => Some(stub_unused(rt, layout, old, &app_code, &app.table, app.start)?),
                    None => None,
                };
                out.section(&append_section(10, rt_code.as_deref(), code.len(), &app_code)?)
            }
            6 if !app.globals.is_empty() || old.is_some() => {
                out.section(&append_section(6, old, globals.len(), &entries_of(&globals))?)
            }
            9 if !app.table.is_empty() || old.is_some() => {
                out.section(&append_section(9, old, elements.len(), &entries_of(&elements))?)
            }
            11 if !app.data.is_empty() || old.is_some() => {
                out.section(&append_section(11, old, data.len(), &entries_of(&data))?)
            }
            12 if layout.data_count + app.data.len() as u32 > 0 => {
                out.section(&wasm_encoder::DataCountSection { count: layout.data_count + app.data.len() as u32 })
            }
            4 => {
                let mut t = TableSection::new();
                t.table(TableType {
                    element_type: RefType::FUNCREF,
                    table64: false,
                    minimum: (layout.table_size + app.table.len() as u32) as u64,
                    maximum: None,
                    shared: false,
                });
                out.section(&t)
            }
            7 => {
                let mut e = ExportSection::new();
                if let Some(old) = old {
                    for export in wasmparser::ExportSectionReader::new(BinaryReader::new(old, 0))? {
                        let export = export?;
                        if export.name.starts_with(rt_abi::PREFIX) {
                            continue;
                        }
                        let kind = match export.kind {
                            wasmparser::ExternalKind::Func => ExportKind::Func,
                            wasmparser::ExternalKind::Table => ExportKind::Table,
                            wasmparser::ExternalKind::Memory => ExportKind::Memory,
                            wasmparser::ExternalKind::Global => ExportKind::Global,
                            _ => bail!("unsupported export kind in plinth-rt"),
                        };
                        e.export(export.name, kind, export.index);
                    }
                }
                out.section(&e)
            }
            8 => match app.start.or(layout.rt_start) {
                Some(function_index) => out.section(&wasm_encoder::StartSection { function_index }),
                None => out,
            },
            _ => match old {
                Some(c) => out.section(&RawSection { id, data: c }),
                None => out,
            },
        };
        Ok(())
    };

    // Standard sections in order. Custom sections stay after the standard
    // section that they followed in the runtime module.
    let mut pending_custom: Vec<&Range<usize>> = Vec::new();
    let flush_until = |out: &mut Module, rank: usize, emitted: &mut Vec<u8>| -> Result<()> {
        for &id in &ORDER[..rank] {
            if !emitted.contains(&id) {
                emit_standard(out, id)?;
                emitted.push(id);
            }
        }
        Ok(())
    };
    for (id, range) in &rt_sections.sections {
        if *id == 0 {
            pending_custom.push(range);
            continue;
        }
        let rank = ORDER.iter().position(|o| o == id).context("unknown section id")? + 1;
        flush_until(&mut out, rank, &mut emitted)?;
        for r in pending_custom.drain(..) {
            emit_custom(&mut out, &rt_sections.bytes[r.clone()])?;
        }
    }
    flush_until(&mut out, ORDER.len(), &mut emitted)?;
    for r in pending_custom.drain(..) {
        emit_custom(&mut out, &rt_sections.bytes[r.clone()])?;
    }

    let bytes = out.finish();
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&bytes)
        .context("the linked module is not valid")?;
    Ok(bytes)
}

/// The body of a stub: no locals, `unreachable`, `end`.
const STUB_BODY: [u8; 4] = [3, 0x00, 0x00, 0x0B];

/// Returns the runtime code section content, with each runtime function that
/// the app cannot reach replaced by a stub.
///
/// The roots are the runtime's own exports (except `__plinth_rt_*`), its start
/// function, its table entries, and the runtime functions that the app code
/// calls or puts in the table. A function that a root calls, or takes with
/// `ref.func`, is reachable too. `call_indirect` can only reach table entries,
/// and all of them are roots.
fn stub_unused(rt: &[u8], layout: &Layout, rt_code: &[u8], app_code: &[u8], app_table: &[u32], app_start: Option<u32>) -> Result<Vec<u8>> {
    let mut imported = 0u32;
    let mut roots: Vec<u32> = Vec::new();
    for payload in Parser::new(0).parse_all(rt) {
        match payload? {
            Payload::ImportSection(r) => {
                for imp in r.into_imports() {
                    if let TypeRef::Func(_) = imp?.ty {
                        imported += 1;
                    }
                }
            }
            Payload::ExportSection(r) => {
                for e in r {
                    let e = e?;
                    if e.kind == wasmparser::ExternalKind::Func && !e.name.starts_with(rt_abi::PREFIX) {
                        roots.push(e.index);
                    }
                }
            }
            Payload::StartSection { func, .. } => roots.push(func),
            Payload::ElementSection(r) => {
                for el in r {
                    match el?.items {
                        wasmparser::ElementItems::Functions(fs) => {
                            for f in fs {
                                roots.push(f?);
                            }
                        }
                        wasmparser::ElementItems::Expressions(_, es) => {
                            for e in es {
                                for op in e?.get_operators_reader() {
                                    if let wasmparser::Operator::RefFunc { function_index } = op? {
                                        roots.push(function_index);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    roots.extend(app_table.iter().copied());
    roots.extend(app_start);
    // The app bodies: the runtime functions that they call.
    let app_entries = app_code;
    let mut r = BinaryReader::new(app_entries, 0);
    while !r.eof() {
        let size = r.read_var_u32()? as usize;
        let start = r.original_position();
        roots.extend(callees(&app_entries[start..start + size])?);
        r.read_bytes(size)?;
    }

    // The runtime bodies, as raw entries in function order.
    let (count, entries) = split_vec(rt_code)?;
    let mut bodies: Vec<&[u8]> = Vec::with_capacity(count as usize);
    let mut r = BinaryReader::new(entries, 0);
    for _ in 0..count {
        let size = r.read_var_u32()? as usize;
        let start = r.original_position();
        bodies.push(&entries[start..start + size]);
        r.read_bytes(size)?;
    }
    debug_assert_eq!(imported + count, layout.func_count);

    let mut live = vec![false; count as usize];
    while let Some(f) = roots.pop() {
        let Some(i) = f.checked_sub(imported) else { continue };
        let Some(slot) = live.get_mut(i as usize) else { continue };
        if !*slot {
            *slot = true;
            roots.extend(callees(bodies[i as usize])?);
        }
    }

    let mut out = Vec::new();
    count.encode(&mut out);
    for (body, live) in bodies.iter().zip(live) {
        if live {
            (body.len() as u32).encode(&mut out);
            out.extend_from_slice(body);
        } else {
            out.extend_from_slice(&STUB_BODY);
        }
    }
    Ok(out)
}

/// The functions that one encoded code body (with its size prefix) calls or
/// takes with `ref.func`.
pub(crate) fn callees_of(encoded: &[u8]) -> Result<Vec<u32>> {
    let mut r = BinaryReader::new(encoded, 0);
    let size = r.read_var_u32()? as usize;
    let start = r.original_position();
    callees(&encoded[start..start + size])
}

/// The functions that one code body calls or takes with `ref.func`.
fn callees(body: &[u8]) -> Result<Vec<u32>> {
    let body = wasmparser::FunctionBody::new(BinaryReader::new(body, 0));
    let mut out = Vec::new();
    for op in body.get_operators_reader()? {
        match op? {
            wasmparser::Operator::Call { function_index }
            | wasmparser::Operator::ReturnCall { function_index }
            | wasmparser::Operator::RefFunc { function_index } => out.push(function_index),
            _ => {}
        }
    }
    Ok(out)
}

fn emit_custom(out: &mut Module, content: &[u8]) -> Result<()> {
    // The content of a custom section starts with its name.
    let mut r = BinaryReader::new(content, 0);
    let name = r.read_string()?;
    let data = &content[r.original_position()..];
    // The name section refers to the runtime's functions only; it stays valid.
    out.section(&wasm_encoder::CustomSection { name: name.into(), data: data.into() });
    Ok(())
}

/// Wraps a linked core module as a `plinth:app` component (SPEC.md §5.1 step 9).
pub fn componentize(core: &[u8]) -> Result<Vec<u8>> {
    wit_component::ComponentEncoder::default()
        .module(core)
        .context("read the linked module")?
        .validate(true)
        .encode()
        .context("encode the component")
}

/// The runtime module that this compiler was built with.
pub fn runtime() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/plinth_rt.wasm"))
}
