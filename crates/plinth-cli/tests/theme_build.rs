//! e2e: the `[theme]` table of `plinth.toml` (`docs/ui.md`, "Theming an
//! app"). `plinth build` carries it into the manifest, rejects a bad one with
//! a message that names the problem, and an app with no `[theme]` has a
//! manifest with no `theme` at all (the package bytes do not change).
//! `plinth validate` runs the same check on a built package.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("plinth-theme-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("app")).unwrap();
    dir
}

/// A copy of `examples/counter` with `extra` added to its `plinth.toml`.
fn project(test: &str, extra: &str) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/counter");
    let dir = scratch(test);
    std::fs::copy(src.join("app/main.tsx"), dir.join("app/main.tsx")).unwrap();
    let toml = std::fs::read_to_string(src.join("plinth.toml")).unwrap();
    std::fs::write(dir.join("plinth.toml"), format!("{toml}\n{extra}")).unwrap();
    dir
}

fn plinth(args: &[&Path]) -> (bool, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_plinth")).args(args).output().expect("run plinth");
    let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    (o.status.success(), text)
}

fn build(dir: &Path) -> (bool, String, PathBuf) {
    let out = dir.join("out.plnt");
    let (ok, text) = plinth(&[Path::new("build"), dir, Path::new("--out"), &out]);
    (ok, text, out)
}

#[test]
fn build_carries_a_valid_theme_into_the_manifest() {
    let dir = project(
        "ok",
        "[theme]\nhover = \"#e0e0ff\"\n[theme.accent]\nlight = \"#0b4fa8\"\ndark = \"#6ea8ff\"\n[theme.on-accent]\nlight = \"#ffffff\"\ndark = \"#0b0b0c\"\n",
    );
    let (ok, text, out) = build(&dir);
    assert!(ok, "{text}");
    let pkg = plinth_package::Package::read(&std::fs::read(&out).unwrap()).unwrap();
    let theme = pkg.manifest.theme.expect("the manifest has the theme");
    assert_eq!(theme.0.len(), 3);
    // `plinth validate` reports the theme.
    let (ok, text) = plinth(&[Path::new("validate"), &out]);
    assert!(ok, "{text}");
    assert!(text.contains("theme: ok (3 tokens set)"), "{text}");
}

#[test]
fn build_without_a_theme_has_no_theme_in_the_manifest() {
    let dir = project("none", "");
    let (ok, text, out) = build(&dir);
    assert!(ok, "{text}");
    let bytes = std::fs::read(&out).unwrap();
    let pkg = plinth_package::Package::read(&bytes).unwrap();
    assert!(pkg.manifest.theme.is_none());
    let manifest = String::from_utf8(pkg.manifest_bytes().unwrap().into_owned()).unwrap();
    assert!(!manifest.contains("theme"), "{manifest}");
    // Two builds give the same bytes.
    let (_, _, out2) = build(&dir);
    assert_eq!(bytes, std::fs::read(&out2).unwrap());
}

#[test]
fn build_rejects_a_bad_theme_with_a_clear_message() {
    // An unknown key.
    let (ok, text, _) = build(&project("unknown", "[theme]\nbrand = \"#112233\"\n"));
    assert!(!ok);
    assert!(text.contains("unknown key `brand`"), "{text}");
    // A bad hex value.
    let (ok, text, _) = build(&project("hex", "[theme]\ntext = \"navy\"\n"));
    assert!(!ok);
    assert!(text.contains("`[theme] text`") && text.contains("hex color"), "{text}");
    // Text that is too faint on the background: names the pair, the ratio and the least.
    let (ok, text, _) = build(&project("contrast", "[theme]\ntext = \"#eeeeee\"\n"));
    assert!(!ok);
    assert!(text.contains("`text` on `background`") && text.contains("light") && text.contains("4.5:1"), "{text}");
}

#[test]
fn validate_rejects_a_package_with_a_failing_theme() {
    // Build a good package, then change its theme in the manifest.
    let dir = project("validate", "");
    let (ok, text, out) = build(&dir);
    assert!(ok, "{text}");
    let mut pkg = plinth_package::Package::read(&std::fs::read(&out).unwrap()).unwrap();
    let mut theme = plinth_package::theme::ThemeOverride::default();
    theme.0.insert("text".into(), plinth_package::theme::ThemeColor::Same("#eeeeee".into()));
    pkg.manifest.theme = Some(theme);
    let bad = dir.join("bad.plnt");
    std::fs::write(&bad, pkg.write().unwrap()).unwrap();
    let (ok, text) = plinth(&[Path::new("validate"), &bad]);
    assert!(!ok, "{text}");
    assert!(text.contains("the package theme is not valid") && text.contains("`text` on `background`"), "{text}");
}
