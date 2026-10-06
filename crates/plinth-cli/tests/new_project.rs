//! e2e: `plinth new` output type-checks and builds (SPEC.md §13).
use std::process::Command;

fn plinth_bin() -> &'static str {
    env!("CARGO_BIN_EXE_plinth")
}

#[test]
fn new_project_checks_and_builds() {
    let dir = std::env::temp_dir().join(format!("plinth-new-e2e-{}", std::process::id()));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }

    let status = Command::new(plinth_bin()).arg("new").arg(&dir).status().expect("run plinth new");
    assert!(status.success(), "plinth new failed");

    // The generated app/ has the list + detail screens, split across files.
    for rel in ["app/main.tsx", "app/model.ts", "app/tasks.tsx", "app/detail.tsx", "plinth.toml", "README.md"] {
        assert!(dir.join(rel).exists(), "missing {rel}");
    }
    let toml = std::fs::read_to_string(dir.join("plinth.toml")).unwrap();
    assert!(toml.contains("store.kv"), "plinth.toml should declare the store.kv capability");

    let check = Command::new(plinth_bin()).arg("check").arg(&dir).output().expect("run plinth check");
    assert!(check.status.success(), "plinth check failed: {}", String::from_utf8_lossy(&check.stderr));
    let check_out = String::from_utf8_lossy(&check.stdout);
    assert!(check_out.contains("0 warnings"), "expected 0 warnings, got: {check_out}");

    let build = Command::new(plinth_bin()).arg("build").arg(&dir).output().expect("run plinth build");
    assert!(build.status.success(), "plinth build failed: {}", String::from_utf8_lossy(&build.stderr));

    std::fs::remove_dir_all(&dir).ok();
}
