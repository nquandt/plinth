//! Pulls the linked core module out of a `plinth:app` component (SPEC.md
//! §9.2, §5.1 step 9). `plinth-link::componentize` wraps one core module with
//! no adapters, so the component holds exactly one nested module: this finds
//! it and returns its raw bytes, ready for `wasmi::Module::new`.

use anyhow::{Result, bail};
use wasmparser::{Parser, Payload};

/// The exports a linked core module must have (SPEC.md §10.4, §10.5):
/// the two entry points and the two things the host needs to pass
/// buffers (`list<u8>` arguments and results).
const REQUIRED_EXPORTS: [&str; 4] = ["init", "on-event", "memory", "cabi_realloc"];

/// Finds the nested core module, inside `component`, that exports `init`,
/// `on-event`, `memory` and `cabi_realloc`, and returns its raw bytes.
pub fn extract_core_module(component: &[u8]) -> Result<Vec<u8>> {
    for payload in Parser::new(0).parse_all(component) {
        if let Payload::ModuleSection { unchecked_range, .. } = payload? {
            let module = &component[unchecked_range];
            if module_has_exports(module)? {
                return Ok(module.to_vec());
            }
        }
    }
    bail!("the component has no core module exporting init, on-event, memory, and cabi_realloc")
}

/// True when `module`'s export section has every name in `REQUIRED_EXPORTS`.
fn module_has_exports(module: &[u8]) -> Result<bool> {
    let mut names: std::collections::HashSet<&str> = Default::default();
    for payload in Parser::new(0).parse_all(module) {
        if let Payload::ExportSection(reader) = payload? {
            for export in reader {
                names.insert(export?.name);
            }
        }
    }
    Ok(REQUIRED_EXPORTS.iter().all(|n| names.contains(*n)))
}

#[cfg(all(test, feature = "wasmi"))]
mod tests {
    use super::*;

    /// A minimal core module with the four required exports, wrapped as a
    /// component the same way `plinth-link::componentize` does, round-trips
    /// through `extract_core_module`.
    #[test]
    fn extracts_the_linked_core_module() {
        let wat = r#"
            (module
                (memory (export "memory") 1)
                (func (export "cabi_realloc") (param i32 i32 i32 i32) (result i32) i32.const 0)
                (func (export "init") (param i32 i32))
                (func (export "on-event") (param i32 i32))
            )
        "#;
        let core = wat::parse_str(wat).unwrap();
        let component = plinth_link::link::componentize(&core).unwrap();

        let extracted = extract_core_module(&component).unwrap();
        assert!(module_has_exports(&extracted).unwrap());
    }

    /// A module missing an export is not picked.
    #[test]
    fn rejects_a_module_missing_an_export() {
        let wat = r#"(module (memory (export "memory") 1))"#;
        let core = wat::parse_str(wat).unwrap();
        assert!(!module_has_exports(&core).unwrap());
    }
}
