//! A shipped core minor version is frozen (`docs/COMPATIBILITY.md` §2,
//! PRODUCTION C3): every function of a shipped minor keeps its name and
//! types, the object layout constants keep their values, and no function is
//! added to a shipped minor. `core-abi.snapshot` records the shipped ABI;
//! its first line records the shipped minor it was blessed at.
//!
//! When a new minor ships, raise `rt_abi::SHIPPED_MINOR` and run
//! `PLINTH_BLESS=1 cargo test -p plinth-link --test core_abi_frozen`. Bless
//! only adds the functions of the newly shipped minors; it never changes or
//! removes a line.

use plinth_link::rt_abi::{self, ADDED_IN, CORE_MAJOR, CORE_MINOR, FUNCTIONS, SHIPPED_MINOR};
use std::collections::BTreeSet;
use std::path::PathBuf;

fn snapshot_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("core-abi.snapshot")
}

/// One line for each function: `name (params) -> (results) @minor`.
fn function_lines(up_to: u32) -> Vec<String> {
    FUNCTIONS
        .iter()
        .filter(|(name, _, _)| rt_abi::added_in(name) <= up_to)
        .map(|(name, params, results)| format!("fn {name} {params:?} -> {results:?} @{}", rt_abi::added_in(name)))
        .collect()
}

/// The layout constants that compiled apps depend on.
fn constant_lines() -> Vec<String> {
    vec![
        format!("const CORE_MAJOR = {CORE_MAJOR}"),
        format!("const ARR_F64 = {}", rt_abi::ARR_F64),
        format!("const ARR_I32 = {}", rt_abi::ARR_I32),
        format!("const ARR_REF = {}", rt_abi::ARR_REF),
        format!("const T_STRING = {}", rt_abi::T_STRING),
        format!("const FIRST_USER_TYPE = {}", rt_abi::FIRST_USER_TYPE),
        format!("const HEADER = {}", rt_abi::HEADER),
        format!("const STR_BYTES = {}", rt_abi::STR_BYTES),
        format!("const PREFIX = {}", rt_abi::PREFIX),
    ]
}

#[test]
fn versions_are_consistent() {
    assert!(CORE_MINOR >= SHIPPED_MINOR, "CORE_MINOR ({CORE_MINOR}) is lower than SHIPPED_MINOR ({SHIPPED_MINOR})");
    for (name, minor) in ADDED_IN {
        assert!(*minor <= CORE_MINOR, "`{name}` is added in 1.{minor}, after CORE_MINOR 1.{CORE_MINOR}");
        assert!(FUNCTIONS.iter().any(|(n, _, _)| n == name), "ADDED_IN names `{name}`, which is not in FUNCTIONS");
    }
}

#[test]
fn shipped_minors_are_frozen() {
    let text = std::fs::read_to_string(snapshot_path()).unwrap_or_default();
    let mut lines = text.lines();
    let header = lines.next().unwrap_or("");
    let blessed: u32 = header.strip_prefix("# shipped minor ").and_then(|v| v.trim().parse().ok()).unwrap_or(0);
    let snapshot: Vec<String> = lines.filter(|l| !l.is_empty() && !l.starts_with('#')).map(str::to_owned).collect();

    let current: BTreeSet<String> = function_lines(u32::MAX).into_iter().chain(constant_lines()).collect();
    let changed: Vec<&String> = snapshot.iter().filter(|l| !current.contains(*l)).collect();
    assert!(
        changed.is_empty(),
        "these shipped core ABI items changed or were removed; old apps would no longer link (docs/COMPATIBILITY.md). Add a new function instead:\n{}",
        changed.iter().map(|l| format!("  {l}")).collect::<Vec<_>>().join("\n")
    );

    // A function in a minor that was already shipped when the snapshot was
    // blessed, but not in the snapshot, was added to a frozen minor.
    let known: BTreeSet<&String> = snapshot.iter().collect();
    // No snapshot yet (the first bless): nothing is frozen.
    let added_to_frozen: Vec<String> = if text.is_empty() {
        Vec::new()
    } else {
        function_lines(blessed).into_iter().filter(|l| !known.contains(l)).collect()
    };
    assert!(
        added_to_frozen.is_empty(),
        "core 1.{blessed} is shipped and frozen; these new functions must go into 1.{} (bump CORE_MINOR and use that minor in ADDED_IN):\n{}",
        blessed + 1,
        added_to_frozen.iter().map(|l| format!("  {l}")).collect::<Vec<_>>().join("\n")
    );

    let wanted: Vec<String> = function_lines(SHIPPED_MINOR).into_iter().chain(constant_lines()).filter(|l| !known.contains(l)).collect();
    if blessed == SHIPPED_MINOR && wanted.is_empty() {
        return;
    }
    if std::env::var("PLINTH_BLESS").is_ok() {
        let mut out = format!("# shipped minor {SHIPPED_MINOR}\n# The frozen core ABI (tests/core_abi_frozen.rs). Append only.\n");
        for l in snapshot.iter().chain(wanted.iter()) {
            out.push_str(l);
            out.push('\n');
        }
        std::fs::write(snapshot_path(), out).unwrap();
        return;
    }
    panic!(
        "SHIPPED_MINOR is 1.{SHIPPED_MINOR} but core-abi.snapshot is blessed at 1.{blessed} ({} new items). If 1.{SHIPPED_MINOR} really shipped, run PLINTH_BLESS=1 cargo test -p plinth-link --test core_abi_frozen",
        wanted.len()
    );
}
