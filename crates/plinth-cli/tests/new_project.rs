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

    // `npm run build:web`: the web export (SPEC.md §10.3), folder and single file.
    for single in [false, true] {
        let mut cmd = Command::new(plinth_bin());
        cmd.arg("build").arg(&dir).args(["--target", "web"]);
        if single {
            cmd.arg("--single-file");
        }
        let out = cmd.output().expect("run plinth build --target web");
        assert!(out.status.success(), "plinth build --target web failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    let web = dir.join("dist/web");
    for file in ["index.html", "plinth.js"] {
        assert!(web.join(file).is_file(), "dist/web/{file} is missing");
    }
    let names = |d: &std::path::Path| -> Vec<String> { std::fs::read_dir(d).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect() };
    let in_web = names(&web);
    assert_eq!(in_web.iter().filter(|n| n.starts_with("plinth-core-") && n.ends_with(".wasm")).count(), 1, "one core file: {in_web:?}");
    assert_eq!(in_web.iter().filter(|n| n.ends_with(".plnt")).count(), 1, "the package: {in_web:?}");
    let single = names(&dir.join("dist")).into_iter().find(|n| n.ends_with(".html")).expect("dist/<name>.html");
    let html = std::fs::read_to_string(dir.join("dist").join(single)).unwrap();
    assert!(html.contains("id=\"plinth-package\"") && html.contains("id=\"plinth-core\""));

    std::fs::remove_dir_all(&dir).ok();
}
