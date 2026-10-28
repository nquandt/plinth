//! A started app holds the run lock (`plinth_hub::RunLock`), so that
//! `plinth hub uninstall` refuses while the app runs (`docs/STORAGE.md` S13),
//! and the host gives the app the data quota of the Hub policy.
//!
//! The test starts a real guest without a window. It uses a unique app id and
//! the real data directory of the user (`start_with_policy` has no other), and
//! deletes the folder of that id at the end.

use plinth_host_desktop::{Owner, start_with_policy};
use plinth_runner_wasmtime::Runner;
use plinth_runner_wasmtime::policy::Policy;
use std::path::PathBuf;
use std::sync::Arc;

fn counter_component() -> Vec<u8> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/counter");
    let fs = plinth_compiler::driver::DiskFs { root };
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[]).expect("compile");
    artifact
        .unwrap_or_else(|| {
            let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
            panic!("counter has errors:\n{}", diags.join("\n"))
        })
        .component
}

#[test]
fn a_running_app_holds_the_lock_until_it_stops() {
    let id = format!("plinth.test.runlock.{}", std::process::id());
    let data_dir = plinth_runner_wasmtime::kv::data_dir();
    let runner = Arc::new(Runner::new().unwrap());
    let component = counter_component();

    assert!(plinth_hub::RunLock::exclusive(&data_dir, &id).is_ok(), "nothing runs yet");
    let (port, init) = start_with_policy(runner.clone(), &component, &id, "0.1.0", &Owner::Dev, Policy::new(Vec::<String>::new()), &[]);
    init.expect("the counter starts");
    let err = plinth_hub::RunLock::exclusive(&data_dir, &id).unwrap_err().to_string();
    assert!(err.contains("is running"), "{err}");
    // A second window of the same app also starts.
    let (second, init) = start_with_policy(runner, &component, &id, "0.1.0", &Owner::Dev, Policy::new(Vec::<String>::new()), &[]);
    init.expect("a second window starts");
    drop(port);
    assert!(plinth_hub::RunLock::exclusive(&data_dir, &id).is_err(), "the second window still runs");
    drop(second);
    assert!(plinth_hub::RunLock::exclusive(&data_dir, &id).is_ok(), "all windows are closed");

    let _ = std::fs::remove_dir_all(data_dir.join("apps").join(&id));
}
