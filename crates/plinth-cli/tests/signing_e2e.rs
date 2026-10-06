//! End-to-end CLI flow for publisher keys and package signatures
//! (`docs/HUB.md` §6.1, §6.2, phase H1): `plinth publisher init`, `plinth
//! sign`, `plinth validate`, `plinth hub add`, `plinth hub list`, each run
//! as the real `plinth` binary with `PLINTH_PUBLISHER_DIR` and
//! `PLINTH_HUB_DIR` pointed at temp directories.

use std::path::PathBuf;
use std::process::Command;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("plinth-cli-e2e-{name}-{}-{}", std::process::id(), rand_suffix()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn rand_suffix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64
}

/// Compiles `examples/counter` into a real, unsigned `.plnt` with the
/// given publisher name, so this test exercises the full package (not a
/// fake one).
fn counter_package(publisher: &str) -> Vec<u8> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/counter");
    let fs = plinth_compiler::driver::DiskFs { root };
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[]).expect("compile");
    let artifact = artifact.unwrap_or_else(|| {
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        panic!("counter has errors:\n{}", diags.join("\n"))
    });
    let component = artifact.app;
    let cfg = plinth_package::ProjectConfig::parse(&format!(
        "id = \"com.example.counter\"\nname = \"Counter\"\nversion = \"0.1.0\"\npublisher = \"{publisher}\"\n"
    ))
    .unwrap();
    let manifest = cfg.manifest("1.0", "plinth-rt/1.0", None, &component);
    plinth_package::Package { manifest, component, assets: Vec::new(), signature: None }.write().unwrap()
}

fn plinth() -> Command {
    Command::new(env!("CARGO_BIN_EXE_plinth"))
}

#[test]
fn publisher_init_sign_validate_hub_add_list() {
    let publisher_dir = temp_dir("pub");
    let hub_dir = temp_dir("hub");
    let work_dir = temp_dir("work");
    let plnt_path = work_dir.join("counter.plnt");
    std::fs::write(&plnt_path, counter_package("Acme")).unwrap();

    // `plinth publisher init --name Acme`.
    let out = plinth().env("PLINTH_PUBLISHER_DIR", &publisher_dir).args(["publisher", "init", "--name", "Acme"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Acme"), "{stdout}");
    assert!(stdout.contains("ed25519:"), "{stdout}");

    // `plinth publisher show` prints the same identity.
    let out = plinth().env("PLINTH_PUBLISHER_DIR", &publisher_dir).args(["publisher", "show"]).output().unwrap();
    assert!(out.status.success());
    let show = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(show.contains("Acme"));
    let key_id = show.split_whitespace().find(|w| w.starts_with("ed25519:")).expect("a key id").to_owned();

    // An unsigned package: `plinth validate` says so.
    let out = plinth().args(["validate", plnt_path.to_str().unwrap()]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("unsigned"));

    // `plinth sign` adds signature.json.
    let out = plinth().env("PLINTH_PUBLISHER_DIR", &publisher_dir).args(["sign", plnt_path.to_str().unwrap()]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    // Now `plinth validate` reports the signer.
    let out = plinth().args(["validate", plnt_path.to_str().unwrap()]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains(&format!("signed by Acme ({key_id})")), "{stdout}");

    // `plinth hub add`, then `plinth hub list` shows the signer.
    let out = plinth().env("PLINTH_HUB_DIR", &hub_dir).args(["hub", "add", plnt_path.to_str().unwrap()]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let out = plinth().env("PLINTH_HUB_DIR", &hub_dir).args(["hub", "list"]).output().unwrap();
    assert!(out.status.success());
    let list = String::from_utf8_lossy(&out.stdout);
    assert!(list.contains(&format!("signed by {key_id}")), "{list}");

    std::fs::remove_dir_all(&publisher_dir).ok();
    std::fs::remove_dir_all(&hub_dir).ok();
    std::fs::remove_dir_all(&work_dir).ok();
}

#[test]
fn tampered_package_is_refused_by_hub_add() {
    let publisher_dir = temp_dir("pub2");
    let hub_dir = temp_dir("hub2");
    let work_dir = temp_dir("work2");
    let plnt_path = work_dir.join("counter.plnt");
    std::fs::write(&plnt_path, counter_package("Acme")).unwrap();

    plinth().env("PLINTH_PUBLISHER_DIR", &publisher_dir).args(["publisher", "init", "--name", "Acme"]).output().unwrap();
    plinth().env("PLINTH_PUBLISHER_DIR", &publisher_dir).args(["sign", plnt_path.to_str().unwrap()]).output().unwrap();

    // Flip one byte of the signed package: the digest no longer matches.
    let mut bytes = std::fs::read(&plnt_path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(&plnt_path, &bytes).unwrap();

    let out = plinth().env("PLINTH_HUB_DIR", &hub_dir).args(["hub", "add", plnt_path.to_str().unwrap()]).output().unwrap();
    assert!(!out.status.success());

    std::fs::remove_dir_all(&publisher_dir).ok();
    std::fs::remove_dir_all(&hub_dir).ok();
    std::fs::remove_dir_all(&work_dir).ok();
}
