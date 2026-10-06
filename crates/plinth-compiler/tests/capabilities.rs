//! The shared capability map (`docs/HUB.md` §7.1, §12.3; `SPEC.md` §11):
//! reachable capabilities come from an app module's imports, and the host
//! refuses to load a module that reaches a capability its manifest does
//! not declare.

use plinth_compiler::driver::DiskFs;
use plinth_compiler::{cores, split};
use std::path::PathBuf;

fn example(name: &str) -> plinth_compiler::driver::DiskFs {
    DiskFs { root: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(name) }
}

#[test]
fn the_notes_app_reaches_exactly_store_kv() {
    let fs = example("notes");
    let caps = vec!["store.kv".to_string()];
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &caps).expect("compile");
    let art = artifact.unwrap_or_else(|| panic!("notes has errors: {:?}", front.diags));
    let reachable = split::reachable_capabilities(&art.app).unwrap();
    assert_eq!(reachable, ["store.kv".to_string()].into_iter().collect());
}

#[test]
fn the_counter_app_reaches_no_capability() {
    let fs = example("counter");
    let (front, artifact) = plinth_compiler::compile(&fs).expect("compile");
    let art = artifact.unwrap_or_else(|| panic!("counter has errors: {:?}", front.diags));
    let reachable = split::reachable_capabilities(&art.app).unwrap();
    assert!(reachable.is_empty(), "the counter should reach no capability, got {reachable:?}");
}

#[test]
fn a_manifest_without_store_kv_is_refused_at_load() {
    let fs = example("notes");
    let caps = vec!["store.kv".to_string()];
    let (_, artifact) = plinth_compiler::compile_with_capabilities(&fs, &caps).expect("compile");
    let art = artifact.expect("notes compiles");

    // The app reaches `store.kv`, but a hand-edited manifest declares
    // nothing: the load-time check must refuse it (`docs/HUB.md` §7.1).
    let err = format!("{:#}", split::check_capabilities(&art.app, &[]).unwrap_err());
    assert!(err.contains("store.kv"), "error should name the missing capability: {err}");

    // With the capability declared, the check passes and the app links.
    split::check_capabilities(&art.app, &caps).unwrap();
    assert!(cores::link_app(&art.app).is_ok());
}

#[test]
fn declared_but_not_used_is_reported() {
    let fs = example("counter");
    // The counter declares `clipboard.write` but never calls it.
    let caps = vec!["clipboard.write".to_string()];
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &caps).expect("compile");
    let art = artifact.unwrap_or_else(|| panic!("counter has errors: {:?}", front.diags));
    let reachable = split::reachable_capabilities(&art.app).unwrap();
    let unused: Vec<&String> = caps.iter().filter(|c| !reachable.contains(c.as_str())).collect();
    assert_eq!(unused, vec![&"clipboard.write".to_string()]);
}
