//! `wit/plinth/app.wit` is append-only inside major version 1
//! (`docs/COMPATIBILITY.md` §3): a core imports these functions by name and
//! type, and a host must link every old core. This test keeps a snapshot of
//! every item (each function, type, enum, record, variant and world entry,
//! with comments removed and whitespace collapsed) in
//! `wit/plinth/app.wit.snapshot` and fails when an item in the snapshot
//! changed or was removed.
//!
//! A new item also fails the test until it is added to the snapshot: run
//! `PLINTH_BLESS=1 cargo test -p plinth-link --test wit_append_only`. Bless
//! only adds new items; it never removes or changes an old line.

use std::collections::BTreeSet;
use std::path::PathBuf;

fn wit_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../wit/plinth")
}

/// Removes `//` comments (also `///` doc comments) from WIT source.
fn strip_comments(src: &str) -> String {
    src.lines().map(|l| l.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n")
}

/// Every item of every interface and world, as `<block>: <item text>`. An
/// item ends at a `;` at depth 1, or at the `}` that closes a nested type
/// (`enum`, `record`, `variant`, `flags`, `resource`) back at depth 1.
fn items(src: &str) -> Vec<String> {
    let src = strip_comments(src);
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut block = String::new();
    let mut header = String::new();
    let mut cur = String::new();
    for c in src.chars() {
        match c {
            '{' => {
                if depth == 0 {
                    block = header.split_whitespace().collect::<Vec<_>>().join(" ");
                    header.clear();
                } else {
                    cur.push(c);
                }
                depth += 1;
            }
            '}' => {
                depth = depth.saturating_sub(1);
                if depth >= 1 {
                    cur.push(c);
                }
                if depth == 1 && !cur.trim().is_empty() {
                    out.push(format!("{block}: {}", normalize(&cur)));
                    cur.clear();
                } else if depth == 0 {
                    cur.clear();
                }
            }
            ';' if depth == 0 => header.clear(), // `package plinth:app@1.0.0;`
            ';' if depth == 1 => {
                if !cur.trim().is_empty() {
                    out.push(format!("{block}: {}", normalize(&cur)));
                }
                cur.clear();
            }
            _ if depth == 0 => header.push(c),
            _ => cur.push(c),
        }
    }
    out
}

fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn app_wit_is_append_only() {
    let src = std::fs::read_to_string(wit_dir().join("app.wit")).unwrap();
    assert!(src.contains("package plinth:app@1.0.0;"), "the WIT package version must stay 1.0.0 in major version 1 (docs/COMPATIBILITY.md §3)");
    let current: Vec<String> = items(&src);
    let current_set: BTreeSet<&String> = current.iter().collect();
    let snap_path = wit_dir().join("app.wit.snapshot");
    let snap_text = std::fs::read_to_string(&snap_path).unwrap_or_default();
    let snapshot: Vec<String> = snap_text.lines().filter(|l| !l.is_empty() && !l.starts_with('#')).map(str::to_owned).collect();

    let missing: Vec<&String> = snapshot.iter().filter(|s| !current_set.contains(s)).collect();
    assert!(
        missing.is_empty(),
        "these app.wit items changed or were removed; a core that uses them would no longer link (docs/COMPATIBILITY.md §3). Add a new item instead:\n{}",
        missing.iter().map(|s| format!("  {s}")).collect::<Vec<_>>().join("\n")
    );

    let known: BTreeSet<&String> = snapshot.iter().collect();
    let new: Vec<&String> = current.iter().filter(|c| !known.contains(c)).collect();
    if new.is_empty() {
        return;
    }
    if std::env::var("PLINTH_BLESS").is_ok() {
        let mut text = snap_text.clone();
        if text.is_empty() {
            text.push_str("# Every item of wit/plinth/app.wit (tests/wit_append_only.rs). Append only.\n");
        }
        for n in &new {
            text.push_str(n);
            text.push('\n');
        }
        std::fs::write(&snap_path, text).unwrap();
        return;
    }
    panic!(
        "new app.wit items are not in wit/plinth/app.wit.snapshot. Check that each one comes with a new core minor version (rt_abi ADDED_IN), then run PLINTH_BLESS=1 cargo test -p plinth-link --test wit_append_only:\n{}",
        new.iter().map(|s| format!("  {s}")).collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn the_item_scanner_splits_functions_and_types() {
    let src = "package a:b@1.0.0;\n/// doc\ninterface x {\n  enum e { a, b }\n  f: func(s: string) -> u32; // tail\n  record r { a: u8 }\n}\nworld w { import x; export g: func(); }\n";
    assert_eq!(
        items(src),
        vec!["interface x: enum e { a, b }", "interface x: f: func(s: string) -> u32", "interface x: record r { a: u8 }", "world w: import x", "world w: export g: func()"]
    );
}
