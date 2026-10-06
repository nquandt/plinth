//! Validation, and tools for working on Plinth itself.

use anyhow::{Context as _, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;
use wasmparser::{Parser, Payload};

/// The prefix of every import that an app may use (SPEC.md §4.1, level 3).
const ALLOWED_IMPORT_PREFIX: &str = "plinth:app/";

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("workspace root")
}

/// Builds a hand-written Rust guest crate (M0) into target/plinth/<name>.wasm.
pub fn build_rust_example(name: &str) -> Result<PathBuf> {
    let root = workspace_root();
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(cargo)
        .current_dir(&root)
        .args(["build", "-p", name, "--target", "wasm32-unknown-unknown", "--profile", "wasm"])
        .status()
        .context("run cargo")?;
    if !status.success() {
        bail!("cargo build of `{name}` failed");
    }
    let core_path = root.join("target/wasm32-unknown-unknown/wasm").join(format!("{}.wasm", name.replace('-', "_")));
    let core = std::fs::read(&core_path).with_context(|| format!("read {}", core_path.display()))?;
    let component = componentize(&core)?;
    validate_component(&component)?;
    let out_dir = root.join("target/plinth");
    std::fs::create_dir_all(&out_dir)?;
    let out = out_dir.join(format!("{name}.wasm"));
    std::fs::write(&out, &component)?;
    Ok(out)
}

pub fn componentize(core: &[u8]) -> Result<Vec<u8>> {
    wit_component::ComponentEncoder::default()
        .module(core)
        .context("read the core module")?
        .validate(true)
        .encode()
        .context("encode the component")
}

/// Validates a `.plnt` package, a bare app module or a bare component, and
/// prints its capability report (`docs/HUB.md` §7.1).
pub fn validate_file(path: &Path) -> Result<()> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    if plinth_package::is_package(&bytes) {
        let pkg = plinth_package::Package::read(&bytes)?;
        if pkg.manifest.ui_api.split('.').next() != plinth_protocol::UI_API_VERSION.split('.').next() {
            bail!("the package needs UI API {}, but this host has {}", pkg.manifest.ui_api, plinth_protocol::UI_API_VERSION);
        }
        validate_entry(&pkg.component, &pkg.manifest.capabilities)
    } else {
        validate_entry(&bytes, &[])
    }
}

/// An app module must link into this runtime (SPEC.md §10.1), reaching
/// only declared capabilities (`docs/HUB.md` §7.1 rule 1); a component
/// must import only the plinth:app world.
fn validate_entry(bytes: &[u8], declared: &[plinth_package::Capability]) -> Result<()> {
    use plinth_compiler::{cores, split};
    if split::is_app_module(bytes) {
        print_capability_report(bytes, declared)?;
        let names: Vec<String> = declared.iter().map(|c| c.name.clone()).collect();
        split::check_capabilities(bytes, &names)?;
        let component = cores::link_app(bytes)?;
        validate_component(&component)
    } else {
        validate_component(bytes)
    }
}

/// Prints the declared, reachable and "declared but not used" capability
/// lists for an app module (`docs/HUB.md` §7.1).
fn print_capability_report(bytes: &[u8], declared: &[plinth_package::Capability]) -> Result<()> {
    use plinth_compiler::split;
    let reachable = split::reachable_capabilities(bytes)?;
    println!("capabilities:");
    if declared.is_empty() {
        println!("  declared: none");
    } else {
        println!("  declared:");
        for c in declared {
            println!("    {} - {}", c.name, c.rationale);
        }
    }
    if reachable.is_empty() {
        println!("  reachable: none");
    } else {
        println!("  reachable: {}", reachable.iter().cloned().collect::<Vec<_>>().join(", "));
    }
    let unused: Vec<&str> = declared.iter().map(|c| c.name.as_str()).filter(|n| !reachable.contains(*n)).collect();
    if !unused.is_empty() {
        println!("  declared but not used: {}", unused.join(", "));
    }
    Ok(())
}

/// Rejects every top-level component import that is not in the plinth:app
/// world, `wasi:*` included (SPEC.md §4.1, level 3).
pub fn validate_component(component: &[u8]) -> Result<()> {
    wasmparser::Validator::new().validate_all(component).context("the artifact is not valid Wasm")?;
    let mut depth = 0u32;
    let mut is_component = false;
    for payload in Parser::new(0).parse_all(component) {
        match payload? {
            Payload::Version { encoding, .. } => {
                if depth == 0 {
                    is_component = encoding == wasmparser::Encoding::Component;
                }
                depth += 1;
            }
            Payload::End(_) => depth = depth.saturating_sub(1),
            Payload::ComponentImportSection(reader) if depth == 1 => {
                for import in reader {
                    let name = import?.name.name;
                    if !name.starts_with(ALLOWED_IMPORT_PREFIX) {
                        bail!("the artifact imports `{name}`, which is not in the plinth:app world");
                    }
                }
            }
            _ => {}
        }
    }
    if !is_component {
        bail!("the artifact is a core module, not a component");
    }
    Ok(())
}
