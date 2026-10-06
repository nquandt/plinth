//! Builds `plinth-rt` for wasm32 and places it in OUT_DIR, so the compiler
//! can embed it. Set `PLINTH_RT_WASM` to use a prebuilt file instead.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("plinth_rt.wasm");
    println!("cargo:rerun-if-env-changed=PLINTH_RT_WASM");
    if let Ok(prebuilt) = std::env::var("PLINTH_RT_WASM") {
        std::fs::copy(&prebuilt, &out).unwrap_or_else(|e| panic!("copy {prebuilt}: {e}"));
        return;
    }
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    for dir in ["crates/plinth-rt", "crates/plinth-protocol", "wit/plinth"] {
        println!("cargo:rerun-if-changed={}", root.join(dir).display());
    }
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    // A separate target directory, so the nested build does not wait for
    // the lock of the outer build.
    let target_dir = root.join("target/rt");
    let status = Command::new(cargo)
        .current_dir(&root)
        .args(["build", "-p", "plinth-rt", "--target", "wasm32-unknown-unknown", "--profile", "wasm", "--target-dir"])
        .arg(&target_dir)
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_BUILD_TARGET")
        .status()
        .expect("run cargo for plinth-rt");
    assert!(status.success(), "building plinth-rt for wasm32 failed");
    std::fs::copy(target_dir.join("wasm32-unknown-unknown/wasm/plinth_rt.wasm"), &out).expect("copy plinth_rt.wasm");
}
