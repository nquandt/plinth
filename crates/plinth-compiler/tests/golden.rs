//! Golden diagnostic tests (SPEC.md §15 M3 exit): one small case per
//! rejected-feature diagnostic code, under `tests/golden/<case>/`.
//!
//! Each case is a folder with `app/main.tsx` (and sometimes `plinth.toml`
//! or `assets/`) plus an `expected.txt` holding the exact rendered
//! diagnostics (the same text `plinth check` prints). Run with
//! `PLINTH_BLESS=1 cargo test -p plinth-compiler --test golden` to
//! (re)write `expected.txt` from the compiler's current output.

use plinth_compiler::driver::DiskFs;
use std::path::{Path, PathBuf};

/// The capability names declared in `<case>/plinth.toml`'s
/// `[[capabilities]]` tables (SPEC.md §11), same minimal parse as
/// `tests/apps.rs` uses for the example apps.
fn declared_capabilities(root: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(root.join("plinth.toml")) else { return Vec::new() };
    let Ok(value) = text.parse::<toml::Value>() else { return Vec::new() };
    value
        .get("capabilities")
        .and_then(|c| c.as_array())
        .map(|caps| caps.iter().filter_map(|c| c.get("name")?.as_str().map(str::to_owned)).collect())
        .unwrap_or_default()
}

/// Normalizes `\` to `/` so the golden files are the same on Windows and
/// elsewhere (file paths appear in the rendered diagnostics).
fn normalize(s: &str) -> String {
    s.replace('\\', "/")
}

fn golden_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

#[test]
fn golden_diagnostics() {
    let root = golden_root();
    let bless = std::env::var("PLINTH_BLESS").is_ok_and(|v| v == "1");
    let mut cases: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", root.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "no golden cases found under {}", root.display());

    let mut failures = Vec::new();
    for case in &cases {
        let name = case.file_name().unwrap().to_string_lossy().into_owned();
        let caps = declared_capabilities(case);
        let fs = DiskFs { root: case.clone() };
        let front = plinth_compiler::driver::frontend_with_capabilities(&fs, &caps);
        let rendered: Vec<String> = front.diags.iter().map(|d| normalize(&front.sources.render(d))).collect();
        let actual = rendered.join("\n\n") + "\n";

        let expected_path = case.join("expected.txt");
        if bless {
            std::fs::write(&expected_path, &actual).unwrap_or_else(|e| panic!("writing {}: {e}", expected_path.display()));
            continue;
        }

        let expected = std::fs::read_to_string(&expected_path).unwrap_or_else(|e| {
            panic!("missing {} ({e}); run with PLINTH_BLESS=1 to create it", expected_path.display())
        });
        // Expected files are checked in with `\n` line endings; tolerate a
        // stray `\r` from a Windows checkout.
        let expected = expected.replace("\r\n", "\n");
        if actual != expected {
            failures.push(format!(
                "case `{name}` differs:\n--- expected ---\n{expected}--- actual ---\n{actual}"
            ));
        }
    }

    if !failures.is_empty() {
        panic!("{} of {} golden case(s) failed:\n\n{}", failures.len(), cases.len(), failures.join("\n"));
    }
}
