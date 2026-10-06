//! App modules hold only the app code; a host links them into an installed
//! core and rejects modules that reach past the core (SPEC.md §10.4, §10.5).

use plinth_compiler::driver::DiskFs;
use plinth_compiler::{cores, link, split};
use std::path::PathBuf;
use wasm_encoder::{CustomSection, EntityType, ImportSection, Module, TypeSection};

fn counter() -> plinth_compiler::Artifact {
    let fs = DiskFs { root: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/counter") };
    plinth_compiler::compile(&fs).unwrap().1.expect("counter compiles")
}

/// A module with one function import that needs this compiler's core.
fn module_importing(module: &str, name: &str) -> Vec<u8> {
    let mut types = TypeSection::new();
    types.ty().function([], []);
    let mut imports = ImportSection::new();
    imports.import(module, name, EntityType::Function(0));
    let mut m = Module::new();
    m.section(&types).section(&imports);
    m.section(&CustomSection { name: split::CORE_SECTION.into(), data: split::core_needed().as_bytes().into() });
    m.finish()
}

fn load_error(app: &[u8]) -> String {
    let rt = link::runtime();
    let layout = link::layout(rt).unwrap();
    format!("{:#}", split::load_app(rt, &layout, app).err().expect("the load must fail"))
}

/// Sets the `MAJOR.MINOR` that an app module needs.
fn needing(mut app: Vec<u8>, version: &str) -> Vec<u8> {
    let (major, minor) = split::app_core_version(&app).unwrap();
    let have = format!("{major}.{minor}");
    assert_eq!(have.len(), version.len(), "keep the length so the section size stays valid");
    let at = app.windows(have.len() + 11).position(|w| w == format!("plinth-core{have}").as_bytes()).unwrap() + 11;
    app[at..at + version.len()].copy_from_slice(version.as_bytes());
    app
}

#[test]
fn the_compiler_and_its_core_agree_on_the_version() {
    assert_eq!(cores::core_version(link::runtime()), cores::parse_version(&split::core_needed()));
    assert!(link::layout(link::runtime()).unwrap().missing.is_empty());
}

#[test]
fn the_counter_app_module_holds_only_app_code() {
    let art = counter();
    assert!(split::is_app_module(&art.app));
    // The counter uses only 1.0 functions, so it runs on every 1.x core.
    assert_eq!(art.runtime, "plinth-core/1.0");
    assert_eq!(split::app_core_version(&art.app), Some((1, 0)));
    // SPEC.md §5.5: the runtime is not in the app; the counter is tiny.
    assert!(art.app.len() <= 4 * 1024, "the counter app module is {} bytes", art.app.len());
    // The host's link step gives a component that runs.
    let component = split::link_app(link::runtime(), &art.app).unwrap();
    assert!(!split::is_app_module(&component));
}

#[test]
fn a_newer_minor_or_another_major_is_rejected_by_this_core() {
    assert!(load_error(&needing(counter().app, "1.9")).contains("needs core 1.9, but this core is 1.4"));
    assert!(load_error(&needing(counter().app, "2.0")).contains("needs core 2.0"));
}

#[test]
fn the_app_module_does_not_depend_on_the_runtime_build() {
    // The app has no runtime table size in it: a core with a different
    // table (another build) links it at its own table end.
    let art = counter();
    let rt = link::runtime();
    let mut layout = link::layout(rt).unwrap();
    let code = split::load_app(rt, &layout, &art.app).unwrap();
    layout.table_size += 7;
    let moved = split::load_app(rt, &layout, &art.app).unwrap();
    assert_eq!(code.table.len(), moved.table.len());
}

#[test]
fn imports_outside_the_core_are_rejected() {
    assert!(load_error(&module_importing("env", "abort")).contains("only `plinth-rt` is allowed"));
    assert!(load_error(&module_importing("plinth-rt", "secret")).contains("this core does not have"));
    // A core function with the wrong type.
    assert!(load_error(&module_importing("plinth-rt", "alloc")).contains("wrong type"));
}

#[test]
fn cores_are_selected_by_major_and_the_highest_usable_minor() {
    let core = |major, minor| cores::Core { version: (major, minor), bytes: Vec::new(), path: None };
    let installed = vec![core(1, 0), core(1, 2), core(1, 5), core(2, 0)];
    assert_eq!(cores::select(&installed, (1, 1)).map(|c| c.version), Some((1, 5)));
    assert_eq!(cores::select(&installed, (1, 5)).map(|c| c.version), Some((1, 5)));
    assert_eq!(cores::select(&installed, (2, 0)).map(|c| c.version), Some((2, 0)));
    assert!(cores::select(&installed, (1, 6)).is_none());
    assert!(cores::select(&installed, (3, 0)).is_none());
}

#[test]
fn install_and_link_through_the_cores_directory() {
    let dir = std::env::temp_dir().join(format!("plinth-cores-{}", std::process::id()));
    // SAFETY: this test is the only one in this binary that reads the variable.
    unsafe { std::env::set_var("PLINTH_CORES_DIR", &dir) };
    let path = cores::install(link::runtime()).unwrap();
    assert!(path.ends_with("1.4/core.wasm") || path.ends_with("1.4\\core.wasm"));
    // Installing the same core again is fine; other contents are not.
    cores::install(link::runtime()).unwrap();
    assert!(cores::install(b"\0asm\x01\0\0\0").is_err());
    assert!(cores::link_app(&counter().app).is_ok());
    let err = format!("{:#}", cores::link_app(&needing(counter().app, "1.9")).unwrap_err());
    assert!(err.contains("plinth core install"), "{err}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_app_needs_the_lowest_core_that_has_its_imports() {
    // `JSON.stringify` uses functions that core 1.1 added; the counter does not.
    let src = r#"import { app, Screen, Text } from "plinth:ui";
import { JSON } from "plinth:core";
function Home() {
  return <Screen title="J"><Text>{JSON.stringify([1, 2])}</Text></Screen>;
}
export default app({ screens: { home: { title: "J", component: Home } } });
"#;
    let mut files = std::collections::HashMap::new();
    files.insert("app/main.tsx".to_owned(), src.to_owned());
    let fs = plinth_compiler::driver::MemFs(files);
    let (front, art) = plinth_compiler::compile(&fs).unwrap();
    let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
    let art = art.unwrap_or_else(|| panic!("the JSON app has errors:
{}", diags.join("
")));
    assert_eq!(art.runtime, "plinth-core/1.1");
    assert_eq!(counter().runtime, "plinth-core/1.0");
}
