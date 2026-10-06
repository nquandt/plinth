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
    let id = split::runtime_id(link::runtime());
    m.section(&CustomSection { name: split::RUNTIME_SECTION.into(), data: id.as_bytes().into() });
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
    assert_eq!(split::app_runtime_id(&art.app).as_deref(), Some(art.runtime.as_str()));
    // SPEC.md §5.5: the runtime is not in the app; the counter is tiny.
    assert!(art.app.len() <= 4 * 1024, "the counter app module is {} bytes", art.app.len());
    // The host's link step gives a component that runs.
    let component = split::link_app(link::runtime(), &art.app).unwrap();
    assert!(!split::is_app_module(&component));
}

#[test]
fn a_module_for_another_runtime_is_rejected() {
    let mut app = counter().app;
    let id = split::runtime_id(link::runtime());
    let pos = app.windows(id.len()).position(|w| w == id.as_bytes()).unwrap();
    app[pos + id.len() - 1] ^= 1;
    assert!(load_error(&app).contains("was built for the runtime"));
}

#[test]
fn imports_outside_the_runtime_are_rejected() {
    assert!(load_error(&module_importing("env", "abort")).contains("only `plinth-rt` is allowed"));
    assert!(load_error(&module_importing("plinth-rt", "secret")).contains("not a runtime function"));
    // A runtime function with the wrong type.
    assert!(load_error(&module_importing("plinth-rt", "alloc")).contains("wrong type"));
}
