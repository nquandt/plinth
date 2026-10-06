//! End-to-end test: build a registry from `examples/counter` and
//! `examples/notes`, serve it, add it as a Hub source, search, install,
//! then publish a new counter version and `update` (`docs/REGISTRY.md`
//! §9). Run once over HTTP and once with a plain folder path.

use plinth_registry::build::{Options, build};
use plinth_registry::source::Source;
use std::path::PathBuf;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("plinth-registry-e2e-{name}-{}-{}", std::process::id(), rand_suffix()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn rand_suffix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64
}

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
    plinth_package::Package { manifest, component, assets: Vec::new(), signature: None }.write().unwrap()
}

fn compile_notes(id: &str, version: &str) -> Vec<u8> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/notes");
    let fs = plinth_compiler::driver::DiskFs { root };
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &["store.kv".to_string()]).expect("compile");
    let artifact = artifact.unwrap_or_else(|| {
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        panic!("notes has errors:\n{}", diags.join("\n"))
    });
    let component = artifact.app;
    let cfg = plinth_package::ProjectConfig::parse(&format!(
        "id = \"{id}\"\nname = \"Notes\"\nversion = \"{version}\"\npublisher = \"me\"\n\n[[capabilities]]\nname = \"store.kv\"\nrationale = \"Save notes on this device.\"\n"
    ))
    .unwrap();
    let manifest = cfg.manifest("1.0", "plinth-rt/1.0", None, &component);
    plinth_package::Package { manifest, component, assets: Vec::new(), signature: None }.write().unwrap()
}

/// Runs the full flow (search, install, publish a new version, update)
/// against `base`, a registry opened from either an HTTP URL or a plain
/// folder path.
fn run_flow(base: &str, reg_dir: &PathBuf) {
    let hub_dir = temp_dir("hub");
    // SAFETY: tests run in separate processes under `cargo test` by
    // default is not guaranteed, so scope env var use to this call only;
    // each test uses its own process-unique hub dir regardless.
    unsafe { std::env::set_var("PLINTH_HUB_DIR", &hub_dir) };
    let hub = plinth_hub::Hub::open_default().unwrap();
    hub.source_add("test", base).unwrap();

    let source = Source::open(base).unwrap();
    let found = source.search("notes").unwrap();
    assert!(found.iter().any(|a| a.id == "com.example.notes"), "search did not find notes: {found:?}");

    let bytes = source.package("com.example.notes", "0.1.0").unwrap();
    let id = hub.add_package(&bytes).unwrap();
    hub.set_registry(&id, "test", base).unwrap();
    let entry = hub.get("com.example.notes").unwrap().unwrap();
    assert_eq!(entry.active_version().unwrap().version, "0.1.0");
    assert_eq!(entry.registry.as_ref().unwrap().name, "test");

    // Publish counter 0.1.0, install it, then publish 0.2.0 into the same
    // registry and update.
    let bytes = compile_counter("com.example.counter", "0.1.0");
    std::fs::write(reg_dir.join("counter-0.1.0.plnt"), &bytes).unwrap();
    build(reg_dir, &Options::default()).unwrap();
    let source = Source::open(base).unwrap();
    let pkg = source.package("com.example.counter", "0.1.0").unwrap();
    let id = hub.add_package(&pkg).unwrap();
    hub.set_registry(&id, "test", base).unwrap();

    std::fs::remove_file(reg_dir.join("counter-0.1.0.plnt")).ok();
    let v2 = compile_counter("com.example.counter", "0.2.0");
    std::fs::create_dir_all(reg_dir.join("incoming")).unwrap();
    std::fs::write(reg_dir.join("incoming/counter-0.2.0.plnt"), &v2).unwrap();
    build(reg_dir, &Options::default()).unwrap();

    // `plinth hub update`'s logic, done directly against the Hub API (the
    // CLI is a thin wrapper tested manually; this exercises the same
    // calls).
    let source = Source::open(base).unwrap();
    let doc = source.app("com.example.counter").unwrap();
    let latest = doc.latest().unwrap();
    assert_eq!(latest.version, "0.2.0");
    let pkg = source.package("com.example.counter", &latest.version).unwrap();
    hub.add_package(&pkg).unwrap();
    hub.set_registry("com.example.counter", "test", base).unwrap();

    let entry = hub.get("com.example.counter").unwrap().unwrap();
    assert_eq!(entry.active_version().unwrap().version, "0.2.0");
    assert_eq!(entry.versions.len(), 2);

    std::fs::remove_dir_all(&hub_dir).ok();
}

#[test]
fn flow_over_http_and_folder() {
    // -- Build a registry with counter and notes. --
    let reg_dir = temp_dir("reg");
    std::fs::write(reg_dir.join("counter.plnt"), compile_counter("com.example.counter-seed", "0.1.0")).unwrap();
    std::fs::write(reg_dir.join("notes.plnt"), compile_notes("com.example.notes", "0.1.0")).unwrap();
    build(&reg_dir, &Options { with_core: true, ..Default::default() }).unwrap();

    // -- Folder path, no server. --
    run_flow(reg_dir.to_str().unwrap(), &reg_dir);

    // -- Served over HTTP on a free port. --
    let port = plinth_registry::serve::serve_background(&reg_dir, 0).unwrap();
    let base = format!("http://127.0.0.1:{port}");
    // Give the listener a moment to accept (the bind already succeeded by
    // the time `serve_background` returns; the first request may still
    // race the thread's `accept_loop` setup, so retry briefly).
    let mut last_err = None;
    for _ in 0..50 {
        match Source::open(&base) {
            Ok(_) => {
                last_err = None;
                break;
            }
            Err(e) => {
                last_err = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    }
    if let Some(e) = last_err {
        panic!("could not reach the served registry: {e:#}");
    }
    run_flow(&base, &reg_dir);

    std::fs::remove_dir_all(&reg_dir).ok();
}

/// The Hub UI's `search` and `install` (`docs/HUB.md` §5.2, phase H3 step
/// 2) through `HubService` with the real source client, against a folder
/// registry: one search across two sources (one of them broken), then an
/// install that records its source.
#[test]
fn hub_service_searches_and_installs_with_hub_sources() {
    use plinth_runner_wasmtime::hub::HubBackend;
    let reg_dir = temp_dir("hubsrc");
    std::fs::write(reg_dir.join("notes.plnt"), compile_notes("com.example.notes", "0.1.0")).unwrap();
    build(&reg_dir, &Options::default()).unwrap();

    let hub = plinth_hub::Hub::open(temp_dir("hubsrc-hub")).unwrap();
    let empty = temp_dir("hubsrc-empty");
    hub.source_add("broken", empty.to_str().unwrap()).unwrap();
    hub.source_add("local", reg_dir.to_str().unwrap()).unwrap();
    let svc = plinth_hub::HubService::with_sources(hub.clone(), std::sync::Arc::new(plinth_registry::HubSources));

    let result: serde_json::Value = serde_json::from_str(&(svc.search("note"))().unwrap()).unwrap();
    assert_eq!(result["hits"][0]["id"], "com.example.notes", "{result}");
    assert_eq!(result["hits"][0]["source"], "local");
    assert_eq!(result["hits"][0]["version"], "0.1.0");
    assert!(result["errors"][0].as_str().unwrap().starts_with("broken:"), "{result}");

    assert_eq!((svc.install("com.example.notes"))().unwrap(), "com.example.notes");
    let entry = hub.get("com.example.notes").unwrap().unwrap();
    assert_eq!(entry.registry.unwrap().name, "local");
    assert!((svc.install("com.example.nothing"))().is_err());

    // Updates (`docs/HUB.md` §9.2): a new version in the folder registry.
    let result: serde_json::Value = serde_json::from_str(&(svc.check_updates(""))().unwrap()).unwrap();
    assert_eq!(result["updates"], serde_json::json!([]), "{result}");
    std::fs::write(reg_dir.join("notes2.plnt"), compile_notes("com.example.notes", "0.2.0")).unwrap();
    build(&reg_dir, &Options::default()).unwrap();
    let result: serde_json::Value = serde_json::from_str(&(svc.check_updates("com.example.notes"))().unwrap()).unwrap();
    assert_eq!(result["updates"][0]["version"], "0.2.0", "{result}");
    assert_eq!((svc.update("com.example.notes"))().unwrap(), "0.2.0");
    assert_eq!(hub.get("com.example.notes").unwrap().unwrap().active_version().unwrap().version, "0.2.0");
}
