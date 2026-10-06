//! Tests for `plinth_registry::build` with real packages compiled from
//! `examples/counter` and `examples/notes` (docs/REGISTRY.md §8).

use plinth_registry::build::{Options, build};
use std::path::{Path, PathBuf};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("plinth-registry-test-{name}-{}-{}", std::process::id(), rand_suffix()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn rand_suffix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64
}

/// Compiles `examples/<example>` and wraps it in a `.plnt` with `id`/`version`.
pub fn compile_package(example: &str, id: &str, version: &str) -> Vec<u8> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(example);
    let fs = plinth_compiler::driver::DiskFs { root };
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &["store.kv".to_string()]).expect("compile");
    let artifact = artifact.unwrap_or_else(|| {
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        panic!("{example} has errors:\n{}", diags.join("\n"))
    });
    let component = artifact.app;
    let cfg = plinth_package::ProjectConfig::parse(&format!(
        "id = \"{id}\"\nname = \"Test App\"\nversion = \"{version}\"\npublisher = \"me\"\n\n[[capabilities]]\nname = \"store.kv\"\nrationale = \"Save notes on this device.\"\n"
    ))
    .unwrap();
    let manifest = cfg.manifest("1.0", "plinth-rt/1.0", None, &component);
    let pkg = plinth_package::Package { manifest, component, assets: Vec::new() };
    pkg.write().unwrap()
}

/// Compiles counter without any declared capability (it needs none), so
/// the capability check in `build` has at least one app with an empty list.
fn compile_counter(id: &str, version: &str) -> Vec<u8> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/counter");
    let fs = plinth_compiler::driver::DiskFs { root };
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[]).expect("compile");
    let artifact = artifact.unwrap_or_else(|| {
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        panic!("counter has errors:\n{}", diags.join("\n"))
    });
    let component = artifact.app;
    let cfg =
        plinth_package::ProjectConfig::parse(&format!("id = \"{id}\"\nname = \"Counter\"\nversion = \"{version}\"\npublisher = \"me\"\n")).unwrap();
    let manifest = cfg.manifest("1.0", "plinth-rt/1.0", None, &component);
    let pkg = plinth_package::Package { manifest, component, assets: Vec::new() };
    pkg.write().unwrap()
}

#[test]
fn build_two_apps_and_is_idempotent() {
    let dir = temp_dir("basic");
    std::fs::write(dir.join("counter.plnt"), compile_counter("com.example.counter", "0.1.0")).unwrap();
    std::fs::create_dir_all(dir.join("incoming")).unwrap();
    std::fs::write(dir.join("incoming/notes.plnt"), compile_package("notes", "com.example.notes", "0.1.0")).unwrap();

    let report = build(&dir, &Options { with_core: true }).unwrap();
    assert_eq!(report.added.len(), 2);

    assert!(dir.join("plinth-registry.json").exists());
    assert!(dir.join("apps/index.json").exists());
    assert!(dir.join("apps/com.example.counter/index.json").exists());
    assert!(dir.join("apps/com.example.notes/index.json").exists());
    assert!(dir.join("cores/index.json").exists());

    let before = snapshot(&dir);

    // Running again on the same input must be a no-op (byte-identical).
    let report2 = build(&dir, &Options { with_core: true }).unwrap();
    assert_eq!(report2.added.len(), 0);
    assert_eq!(report2.unchanged.len(), 2);
    assert_eq!(before, snapshot(&dir), "rebuilding the same input changed the output");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn refuses_a_different_package_with_same_id_and_version() {
    let dir = temp_dir("conflict");
    std::fs::write(dir.join("a.plnt"), compile_counter("com.example.counter", "0.1.0")).unwrap();
    build(&dir, &Options::default()).unwrap();

    // A different package body for the same id@version (a different name
    // changes the manifest bytes, hence the digest).
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/counter");
    let fs = plinth_compiler::driver::DiskFs { root };
    let (_front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[]).unwrap();
    let component = artifact.unwrap().app;
    let cfg = plinth_package::ProjectConfig::parse("id = \"com.example.counter\"\nname = \"Different\"\nversion = \"0.1.0\"\npublisher = \"me\"\n").unwrap();
    let manifest = cfg.manifest("1.0", "plinth-rt/1.0", None, &component);
    let pkg = plinth_package::Package { manifest, component, assets: Vec::new() };
    std::fs::remove_file(dir.join("a.plnt")).ok();
    std::fs::write(dir.join("b.plnt"), pkg.write().unwrap()).unwrap();

    assert!(build(&dir, &Options::default()).is_err());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn adding_a_second_version_appends() {
    let dir = temp_dir("second-version");
    std::fs::write(dir.join("a.plnt"), compile_counter("com.example.counter", "0.1.0")).unwrap();
    build(&dir, &Options::default()).unwrap();
    std::fs::remove_file(dir.join("a.plnt")).ok();
    std::fs::write(dir.join("b.plnt"), compile_counter("com.example.counter", "0.2.0")).unwrap();
    build(&dir, &Options::default()).unwrap();

    let doc_text = std::fs::read_to_string(dir.join("apps/com.example.counter/index.json")).unwrap();
    let doc: plinth_registry::AppDocument = serde_json::from_str(&doc_text).unwrap();
    assert_eq!(doc.versions.len(), 2);
    assert_eq!(doc.versions[0].version, "0.2.0");
    assert_eq!(doc.versions[1].version, "0.1.0");
    std::fs::remove_dir_all(&dir).ok();
}

fn snapshot(dir: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                if rel.ends_with(".json") {
                    out.push((rel, std::fs::read_to_string(&path).unwrap()));
                } else {
                    out.push((rel, format!("<{} bytes>", std::fs::metadata(&path).unwrap().len())));
                }
            }
        }
    }
    walk(dir, dir, &mut out);
    out.sort();
    out
}
