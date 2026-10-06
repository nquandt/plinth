//! The `plinth` command-line tool (SPEC.md §13).
//!
//! M0 has the commands that the hand-written guests need. `check`, `build`
//! and `dev` come with the compiler in M1.

use anyhow::{Context as _, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;
use wasmparser::{Parser, Payload};

const USAGE: &str = "\
usage:
  plinth example <name> [--run]       build examples/<name> into target/plinth/<name>.wasm
  plinth componentize <core.wasm> -o <app.wasm>
                                      wrap a core module as a plinth:app component
  plinth validate <app.wasm>          check that the component imports only plinth:app/*";

/// The prefix of every import that an app may use (SPEC.md §4.1, level 3).
const ALLOWED_IMPORT_PREFIX: &str = "plinth:app/";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["example", name, rest @ ..] => {
            let out = build_example(name)?;
            println!("{}", out.display());
            if rest.contains(&"--run") {
                run_host(&out)?;
            }
            Ok(())
        }
        ["componentize", input, "-o", output] => {
            let component = componentize(&std::fs::read(input).with_context(|| format!("read {input}"))?)?;
            validate(&component)?;
            std::fs::write(output, component).with_context(|| format!("write {output}"))
        }
        ["validate", input] => {
            validate(&std::fs::read(input).with_context(|| format!("read {input}"))?)?;
            println!("ok: {input}");
            Ok(())
        }
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("workspace root")
}

fn cargo() -> Command {
    Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
}

/// Builds a guest crate for wasm32-unknown-unknown and wraps it.
fn build_example(name: &str) -> Result<PathBuf> {
    let root = workspace_root();
    let status = cargo()
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
    validate(&component)?;

    let out_dir = root.join("target/plinth");
    std::fs::create_dir_all(&out_dir)?;
    let out = out_dir.join(format!("{name}.wasm"));
    std::fs::write(&out, &component)?;
    eprintln!("core module {} KiB, component {} KiB", core.len() / 1024, component.len() / 1024);
    Ok(out)
}

fn run_host(artifact: &Path) -> Result<()> {
    let status = cargo()
        .current_dir(workspace_root())
        .args(["run", "--release", "-p", "plinth-host-desktop", "--"])
        .arg(artifact)
        .status()
        .context("run the desktop host")?;
    if !status.success() {
        bail!("the desktop host failed");
    }
    Ok(())
}

/// Wraps a core module that carries a `component-type` section (from
/// wit-bindgen) as a component (SPEC.md §5.1 step 9).
fn componentize(core: &[u8]) -> Result<Vec<u8>> {
    wit_component::ComponentEncoder::default()
        .module(core)
        .context("read the core module")?
        .validate(true)
        .encode()
        .context("encode the component")
}

/// Rejects every top-level component import that is not in the plinth:app
/// world, `wasi:*` included (SPEC.md §4.1, level 3).
fn validate(component: &[u8]) -> Result<()> {
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
        bail!("the artifact is a core module, not a component; run `plinth componentize` first");
    }
    Ok(())
}
