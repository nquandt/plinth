//! App modules hold only the app code; a host links them into its own
//! runtime and rejects modules that reach past the runtime (SPEC.md §10.1).

use plinth_compiler::driver::DiskFs;
use plinth_compiler::{link, split};
use std::path::PathBuf;
use wasm_encoder::{CustomSection, EntityType, ImportSection, Module, TypeSection};

fn counter() -> plinth_compiler::Artifact {
    let fs = DiskFs { root: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/counter") };
    plinth_compiler::compile(&fs).unwrap().1.expect("counter compiles")
}

/// A module with one function import and the runtime section of this
/// runtime.
fn module_importing(module: &str, name: &str) -> Vec<u8> {
    let mut types = TypeSection::new();
    types.ty().function([], []);
    let mut imports = ImportSection::new();
    imports.import(module, name, EntityType::Function(0));
    let mut m = Module::new();
    m.section(&types).section(&imports);
    let id = split::abi_id();
    m.section(&CustomSection { name: split::ABI_SECTION.into(), data: id.as_bytes().into() });
    m.finish()
}

fn load_error(app: &[u8]) -> String {
    let rt = link::runtime();
    let layout = link::layout(rt).unwrap();
    format!("{:#}", split::load_app(rt, &layout, app).err().expect("the load must fail"))
}

#[test]
fn the_counter_app_module_holds_only_app_code() {
    let art = counter();
    assert!(split::is_app_module(&art.app));
    assert_eq!(split::app_abi_id(&art.app).as_deref(), Some(art.runtime.as_str()));
    assert_eq!(art.runtime, "plinth-abi/1");
    // SPEC.md §5.5: the runtime is not in the app; the counter is tiny.
    assert!(art.app.len() <= 4 * 1024, "the counter app module is {} bytes", art.app.len());
    // The host's link step gives a component that runs.
    let component = split::link_app(link::runtime(), &art.app).unwrap();
    assert!(!split::is_app_module(&component));
}

#[test]
fn a_module_for_another_abi_major_is_rejected() {
    let mut app = counter().app;
    let id = split::abi_id();
    let pos = app.windows(id.len()).position(|w| w == id.as_bytes()).unwrap();
    app[pos + id.len() - 1] = b'9';
    assert!(load_error(&app).contains("needs the runtime ABI `plinth-abi/9`"));
}

#[test]
fn the_app_module_does_not_depend_on_the_runtime_build() {
    // The app has no runtime build id and no runtime table size in it: a
    // runtime with a different table (another build of the same ABI) links
    // it at its own table end.
    let art = counter();
    assert!(!art.app.windows(10).any(|w| w == b"plinth-rt/"));
    let rt = link::runtime();
    let mut layout = link::layout(rt).unwrap();
    let code = split::load_app(rt, &layout, &art.app).unwrap();
    layout.table_size += 7;
    let moved = split::load_app(rt, &layout, &art.app).unwrap();
    assert_eq!(code.table.len(), moved.table.len());
}

#[test]
fn imports_outside_the_runtime_are_rejected() {
    assert!(load_error(&module_importing("env", "abort")).contains("only `plinth-rt` is allowed"));
    assert!(load_error(&module_importing("plinth-rt", "secret")).contains("this runtime does not have"));
    // A runtime function with the wrong type.
    assert!(load_error(&module_importing("plinth-rt", "alloc")).contains("wrong type"));
}
