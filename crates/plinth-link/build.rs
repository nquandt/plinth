//! Builds `plinth-rt` for wasm32 and places it in OUT_DIR, so the compiler
//! can embed it. Set `PLINTH_RT_WASM` to use a prebuilt file instead.
//!
//! It builds two blobs: the release one (default features) and a dev one
//! (the `dev` feature, SPEC.md §13) that also registers module-level
//! signals and answers hot-reload snapshot requests. `plinth build` links
//! the release blob, so a release app artifact never contains that code;
//! `plinth dev` links the dev blob.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    println!("cargo:rerun-if-env-changed=PLINTH_RT_WASM");
    println!("cargo:rerun-if-env-changed=PLINTH_RT_WASM_DEV");
    if let Ok(prebuilt) = std::env::var("PLINTH_RT_WASM") {
        std::fs::copy(&prebuilt, out_dir.join("plinth_rt.wasm")).unwrap_or_else(|e| panic!("copy {prebuilt}: {e}"));
        let dev = std::env::var("PLINTH_RT_WASM_DEV").unwrap_or(prebuilt);
        std::fs::copy(&dev, out_dir.join("plinth_rt_dev.wasm")).unwrap_or_else(|e| panic!("copy {dev}: {e}"));
        return;
    }
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    for dir in ["crates/plinth-rt", "crates/plinth-protocol", "wit/plinth"] {
        println!("cargo:rerun-if-changed={}", root.join(dir).display());
    }
    build_rt(&root, &[], &root.join("target/rt"), &out_dir.join("plinth_rt.wasm"));
    build_rt(&root, &["--features", "dev"], &root.join("target/rt-dev"), &out_dir.join("plinth_rt_dev.wasm"));
}

fn build_rt(root: &Path, extra_args: &[&str], target_dir: &Path, out: &Path) {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(cargo)
        .current_dir(root)
        .args(["build", "-p", "plinth-rt", "--target", "wasm32-unknown-unknown", "--profile", "wasm", "--target-dir"])
        .arg(target_dir)
        .args(extra_args)
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_BUILD_TARGET")
        // Export the function table so a browser host can grow it and
        // link app.wasm against it directly, with no static link in the
        // browser (SPEC.md §18.3). Scoped to this target only.
        .env("CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS", "-C link-arg=--export-table")
        .status()
        .expect("run cargo for plinth-rt");
    assert!(status.success(), "building plinth-rt for wasm32 failed");
    std::fs::copy(target_dir.join("wasm32-unknown-unknown/wasm/plinth_rt.wasm"), out).expect("copy plinth_rt.wasm");
}
