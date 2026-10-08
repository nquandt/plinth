//! `plinth build --release`: optimizes the app module with Binaryen
//! `wasm-opt` (SPEC.md §5.1 step 10, `docs/PRODUCTION.md` T7).
//!
//! Binaryen is built into `plinth` (the `wasm-opt` crate), so there is no
//! separate tool to install (SPEC.md §13.1). The optimizer runs `-Oz` with
//! only the Wasm features that the compiler itself uses. After the run,
//! `check` makes sure that the module still has the contract of an app
//! module (SPEC.md §10.4): the same imports, the same custom sections, and
//! a link into the built-in core.

use anyhow::{Context as _, Result, bail, ensure};
use std::collections::BTreeMap;
use wasm_opt::{Feature, OptimizationOptions};
use wasmparser::{Parser, Payload, TypeRef};

/// Optimizes the app module `app` for size and checks the result.
pub fn optimize_app(app: &[u8]) -> Result<Vec<u8>> {
    let out = run_wasm_opt(app)?;
    check(app, &out).context("the optimized app module is not safe to use")?;
    Ok(out)
}

/// Runs Binaryen on `app`. The `wasm-opt` crate reads and writes files, so
/// the module goes through a private temporary folder.
fn run_wasm_opt(app: &[u8]) -> Result<Vec<u8>> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("plinth-wasm-opt-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let (input, output) = (dir.join("app.wasm"), dir.join("app.opt.wasm"));
    let result = (|| {
        std::fs::write(&input, app).with_context(|| format!("write {}", input.display()))?;
        let mut opts = OptimizationOptions::new_optimize_for_size_aggressively();
        // Only the features of the compiler's output (`codegen.rs`): the
        // optimizer must not use an instruction that a host engine does not
        // have. Bulk memory: `memory.init` for literals; TruncSat:
        // `i32.trunc_sat_f64_s`. Sign extension and mutable globals are in
        // every engine that Plinth uses (wasmtime, wasmi, browsers).
        opts.mvp_features_only()
            .enable_feature(Feature::MutableGlobals)
            .enable_feature(Feature::SignExt)
            .enable_feature(Feature::BulkMemory)
            .enable_feature(Feature::TruncSat);
        opts.run(&input, &output).context("wasm-opt failed")?;
        std::fs::read(&output).with_context(|| format!("read {}", output.display()))
    })();
    std::fs::remove_dir_all(&dir).ok();
    result
}

/// The parts of a module that the hosts read.
#[derive(Default)]
struct Contract {
    /// `(module, name)` to a description of the import type.
    imports: BTreeMap<(String, String), String>,
    /// Name to content.
    custom: BTreeMap<String, Vec<u8>>,
    exports: u32,
}

fn contract(bytes: &[u8]) -> Result<Contract> {
    let mut c = Contract::default();
    let mut types: Vec<wasmparser::FuncType> = Vec::new();
    for payload in Parser::new(0).parse_all(bytes) {
        match payload? {
            Payload::TypeSection(r) => {
                for group in r {
                    for ty in group?.into_types() {
                        match ty.composite_type.inner {
                            wasmparser::CompositeInnerType::Func(f) => types.push(f),
                            _ => bail!("a non-function type"),
                        }
                    }
                }
            }
            Payload::ImportSection(r) => {
                for imp in r.into_imports() {
                    let imp = imp?;
                    let ty = match imp.ty {
                        TypeRef::Func(t) => {
                            let f = types.get(t as usize).context("bad import type")?;
                            format!("func {:?} -> {:?}", f.params(), f.results())
                        }
                        other => format!("{other:?}"),
                    };
                    c.imports.insert((imp.module.to_string(), imp.name.to_string()), ty);
                }
            }
            Payload::ExportSection(r) => c.exports += r.count(),
            Payload::CustomSection(s) => {
                c.custom.insert(s.name().to_string(), s.data().to_vec());
            }
            _ => {}
        }
    }
    Ok(c)
}

/// Checks that `after` keeps the contract of the app module `before`.
fn check(before: &[u8], after: &[u8]) -> Result<()> {
    let (b, a) = (contract(before)?, contract(after)?);
    // Each custom section (`plinth-core` gives the core version) is kept.
    for (name, data) in &b.custom {
        ensure!(a.custom.get(name) == Some(data), "the custom section `{name}` is missing or changed");
    }
    // The optimizer may remove a function import that no code calls any
    // more. It must not add an import or change one: a host matches the
    // imports by name and type, and the web host grows the table by the
    // minimum of the table import.
    for (key, ty) in &a.imports {
        match b.imports.get(key) {
            Some(t) => ensure!(t == ty, "the import `{}` `{}` changed from {t} to {ty}", key.0, key.1),
            None => bail!("a new import `{}` `{}`", key.0, key.1),
        }
    }
    for name in ["memory", "table", plinth_link::split::TABLE_BASE] {
        let key = (plinth_link::split::RT_MODULE.to_string(), name.to_string());
        ensure!(a.imports.contains_key(&key), "the import `{name}` is missing");
    }
    ensure!(a.exports == 0, "the module has exports");
    // It links into the built-in core the same way a host links it.
    plinth_link::split::link_app(plinth_compiler::link::runtime(), after).context("the module does not link")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A module with only the custom section `plinth-core` (the end-to-end
    /// tests in `tests/release_build.rs` use real app modules).
    fn with_core_section(data: &str) -> Vec<u8> {
        let name = b"plinth-core";
        let mut m = vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x00];
        m.push((1 + name.len() + data.len()) as u8);
        m.push(name.len() as u8);
        m.extend_from_slice(name);
        m.extend_from_slice(data.as_bytes());
        m
    }

    #[test]
    fn check_refuses_a_changed_custom_section() {
        let err = check(&with_core_section("1.12"), &with_core_section("1.11")).unwrap_err();
        assert!(format!("{err:#}").contains("plinth-core"), "{err:#}");
    }

    #[test]
    fn check_refuses_a_module_with_no_rt_imports() {
        let err = check(&with_core_section("1.12"), &with_core_section("1.12")).unwrap_err();
        assert!(format!("{err:#}").contains("import `memory` is missing"), "{err:#}");
    }
}
