//! Performance budgets in the gate (`docs/PRODUCTION.md` R6,
//! `docs/guides/performance.md` §9). The budgets are in `tests/budgets.toml`.
//!
//! - `app_size`: the size of `app.wasm` (the app code of a `.plnt`, debug
//!   build, no `--release`) of each example. Deterministic: a tight budget.
//! - `core_size`: the size of the core. Recorded, not a limit (SPEC.md §5.5:
//!   the runtime has no size limit). A large change prints a note.
//! - `memory`: the guest linear memory after `init` and after a fixed
//!   sequence of UI events, for some examples. Deterministic: a tight budget.
//! - `start`: the time of `Runner::load` plus `init` to the first commit, for
//!   the same examples. Timings are noisy, so the budget is loose (see
//!   `measure_start`).
//!
//! A failure names the metric, the budget, the value and how to bless. To
//! accept an intended change, run
//! `PLINTH_BLESS=1 cargo test -p plinth-compiler --test budgets` and read the
//! printed diff. Bless writes new budgets from the values of this run.

use plinth_compiler::driver::DiskFs;
use plinth_protocol::{ControlKind, Event, NodeId, Value, Writer, event, prop};
use plinth_runner_wasmtime::{Limits, Runner};
use plinth_ui::tree::Tree;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The margin over the measured value for a deterministic budget.
const SIZE_MARGIN: f64 = 0.10;
/// The examples of the start-time and memory budgets.
const RUN_APPS: &[&str] = &["counter", "todo", "budget", "hub", "big-list"];
/// UI events for the memory budget after events.
const EVENTS: usize = 300;
/// Rounds of the start-time measurement; the median of the rounds is used.
const START_ROUNDS: usize = 5;
/// A start-time budget is this multiple of the median that bless measured.
const START_FACTOR: f64 = 3.0;
/// The lowest budget for `load` and for `init`, in ms: a very short time is
/// mostly noise.
const LOAD_FLOOR_MS: f64 = 100.0;
const INIT_FLOOR_MS: f64 = 10.0;
/// The memory page of Wasm.
const PAGE: u64 = 64 << 10;

/// The tests of this file run one at a time: a compile in a parallel test
/// would slow the start-time samples.
fn serial() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn budgets_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/budgets.toml")
}

fn blessing() -> bool {
    std::env::var("PLINTH_BLESS").is_ok_and(|v| v == "1")
}

const BLESS_HINT: &str = "If the change is intended, run `PLINTH_BLESS=1 cargo test -p plinth-compiler --test budgets` and commit tests/budgets.toml (docs/guides/performance.md §9).";

type Table = BTreeMap<String, BTreeMap<String, u64>>;

fn load_budgets() -> Table {
    let Ok(text) = std::fs::read_to_string(budgets_path()) else { return Table::new() };
    let value: toml::Table = text.parse().expect("tests/budgets.toml is not valid TOML");
    let mut out = Table::new();
    for (section, items) in value {
        let Some(items) = items.as_table() else { continue };
        let map = out.entry(section).or_default();
        for (k, v) in items {
            map.insert(k.clone(), v.as_integer().unwrap_or_else(|| panic!("budgets.toml: {k} is not an integer")) as u64);
        }
    }
    out
}

const HEADER: &str = "# Performance budgets (docs/PRODUCTION.md R6, docs/guides/performance.md §9).
# Checked by crates/plinth-compiler/tests/budgets.rs in the gate.
# Do not edit by hand: run
#   PLINTH_BLESS=1 cargo test -p plinth-compiler --test budgets
# and commit the result with the change that caused it.
";

const SECTIONS: &[(&str, &str)] = &[
    ("app_size", "app.wasm of each example, debug build, bytes. Budget = measured + 10 %."),
    ("core_size", "The core, bytes. Recorded, not a limit (SPEC.md §5.5); a change over 10 % prints a note."),
    ("memory", "Guest linear memory after init and after 300 fixed UI events, bytes. Budget = measured + 10 %, up to a 64 KiB page."),
    ("start_ms", "Start time, ms, debug build: <app>.load = Runner::load (the engine compiles core + app), <app>.init = init to the first commit. Budget = 3 x the median at bless time, at least 100 ms (load) and 10 ms (init)."),
];

fn write_budgets(table: &Table) {
    let mut text = String::from(HEADER);
    for (section, doc) in SECTIONS {
        let Some(items) = table.get(*section) else { continue };
        text.push_str(&format!("\n# {doc}\n[{section}]\n"));
        for (k, v) in items {
            text.push_str(&format!("{} = {v}\n", toml::Value::String(k.clone())));
        }
    }
    std::fs::write(budgets_path(), text).unwrap();
}

/// Writes `new` into the `section` of the budgets file and prints the diff.
fn bless(section: &str, new: BTreeMap<String, u64>) {
    let mut table = load_budgets();
    let old = table.remove(section).unwrap_or_default();
    eprintln!("bless [{section}]:");
    let mut keys: Vec<&String> = old.keys().chain(new.keys()).collect();
    keys.sort();
    keys.dedup();
    let mut changed = 0;
    for k in keys {
        match (old.get(k), new.get(k)) {
            (Some(a), Some(b)) if a == b => {}
            (a, b) => {
                changed += 1;
                let show = |v: Option<&u64>| v.map_or("(none)".to_owned(), |v| v.to_string());
                eprintln!("  {k}: {} -> {}", show(a), show(b));
            }
        }
    }
    if changed == 0 {
        eprintln!("  no change");
    }
    table.insert(section.to_owned(), new);
    write_budgets(&table);
}

fn with_margin(value: u64) -> u64 {
    (value as f64 * (1.0 + SIZE_MARGIN)).ceil() as u64
}

fn examples() -> Vec<(String, PathBuf)> {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut out = Vec::new();
    for e in std::fs::read_dir(&base).unwrap().flatten() {
        let p = e.path();
        if p.join("app/main.tsx").is_file() {
            out.push(p);
        } else if p.is_dir() {
            out.extend(std::fs::read_dir(&p).unwrap().flatten().map(|e| e.path()).filter(|p| p.join("app/main.tsx").is_file()));
        }
    }
    out.sort();
    out.into_iter().map(|p| (p.strip_prefix(&base).unwrap().to_string_lossy().replace('\\', "/"), p)).collect()
}

fn capabilities(root: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(root.join("plinth.toml")).unwrap_or_default();
    text.parse::<toml::Value>()
        .ok()
        .and_then(|v| v.get("capabilities")?.as_array().cloned())
        .map(|caps| caps.iter().filter_map(|c| c.get("name")?.as_str().map(str::to_owned)).collect())
        .unwrap_or_default()
}

fn build(name: &str, root: &Path) -> plinth_compiler::Artifact {
    let caps = capabilities(root);
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&DiskFs { root: root.to_owned() }, &caps).expect("compile");
    artifact.unwrap_or_else(|| panic!("{name} does not build: {:?}", front.diags.iter().map(|d| front.sources.render(d)).collect::<Vec<_>>()))
}

fn build_example(name: &str) -> plinth_compiler::Artifact {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(name);
    build(name, &root)
}

/// Compares each value with its budget. `budget_of` gives the budget from
/// the value for bless; `limit` says if a value is over its budget.
fn check(section: &str, unit: &str, values: &BTreeMap<String, u64>, budget_of: impl Fn(&str, u64) -> u64) {
    if blessing() {
        bless(section, values.iter().map(|(k, v)| (k.clone(), budget_of(k, *v))).collect());
        return;
    }
    let table = load_budgets();
    let budgets = table.get(section).cloned().unwrap_or_default();
    // A GitHub-hosted runner (the macOS job) is a shared machine of an
    // unknown speed: the timing budgets of the bless machine do not apply.
    // The values are printed above.
    if section == "start_ms" && std::env::var("RUNNER_ENVIRONMENT").is_ok_and(|v| v == "github-hosted") {
        eprintln!("note: {section}: not checked on a GitHub-hosted runner");
        return;
    }
    let mut failures = Vec::new();
    for (k, v) in values {
        match budgets.get(k) {
            None => failures.push(format!("{section}.{k}: no budget (value {v} {unit})")),
            Some(b) if v > b => failures.push(format!("{section}.{k}: value {v} {unit} is over the budget {b} {unit} (+{:.1} %)", (*v as f64 / *b as f64 - 1.0) * 100.0)),
            Some(b) => {
                // Far under the budget: a tighter budget would catch more.
                // Not for timings: they move from run to run.
                if section != "start_ms" && budget_of(k, *v) * 4 < *b * 3 {
                    eprintln!("note: {section}.{k}: value {v} {unit} is far under the budget {b} {unit}; bless to tighten it");
                }
            }
        }
    }
    for k in budgets.keys() {
        if !values.contains_key(k) {
            eprintln!("note: {section}.{k}: a budget with no measurement; bless to remove it");
        }
    }
    assert!(failures.is_empty(), "performance budget exceeded:\n  {}\n{BLESS_HINT}", failures.join("\n  "));
}

#[test]
fn app_size_of_every_example() {
    let _serial = serial();
    let mut values = BTreeMap::new();
    for (name, root) in examples() {
        let art = build(&name, &root);
        values.insert(name, art.app.len() as u64);
    }
    eprintln!("app.wasm sizes (debug build):");
    for (k, v) in &values {
        eprintln!("  {k:<24} {v:>7} B");
    }
    check("app_size", "B", &values, |_, v| with_margin(v));
}

#[test]
fn core_size_is_recorded() {
    let _serial = serial();
    let art = build_example("counter");
    let mut values = BTreeMap::new();
    values.insert("raw".to_owned(), plinth_compiler::link::runtime().len() as u64);
    values.insert("linked".to_owned(), art.core_size as u64);
    eprintln!("core: raw {} B, linked {} B", values["raw"], values["linked"]);
    if blessing() {
        bless("core_size", values);
        return;
    }
    // No limit (SPEC.md §5.5, the owner's decision): only a note.
    let recorded = load_budgets().remove("core_size").unwrap_or_default();
    for (k, v) in &values {
        match recorded.get(k) {
            Some(r) if (*v as f64 - *r as f64).abs() > *r as f64 * SIZE_MARGIN => {
                eprintln!("note: core_size.{k}: {v} B, recorded {r} B (more than 10 % change); bless to record it. Not a failure.")
            }
            Some(_) => {}
            None => eprintln!("note: core_size.{k}: not recorded; bless to record it"),
        }
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
}

/// The listeners of all screens, in a fixed order: `(node, event, handler)`.
fn listeners(tree: &Tree) -> Vec<(NodeId, u16, u32)> {
    let mut out = Vec::new();
    let mut stack: Vec<NodeId> = tree.screens().map(|(_, id)| id).collect();
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(n) = tree.get(id) else { continue };
        for &(ev, h) in &n.listeners {
            out.push((id, ev, h));
        }
        stack.extend(n.children.iter().copied());
    }
    out.sort();
    out
}

/// A plausible value for `ev`, as a host sends it.
fn value_for(rng: &mut Rng, tree: &Tree, id: NodeId, ev: u16) -> Value {
    let n = tree.get(id).unwrap();
    match ev {
        event::CHANGE | event::SUBMIT => match n.kind {
            Some(ControlKind::Toggle | ControlKind::Checkbox | ControlKind::Sheet | ControlKind::Dialog) => Value::Bool(rng.below(2) == 1),
            Some(ControlKind::Slider | ControlKind::NumberField) => Value::Number([0.0, 1.0, 5.5, 50.0][rng.below(4)]),
            Some(ControlKind::Picker | ControlKind::Tabs) => {
                let list = n.str_prop(prop::OPTIONS).or(n.str_prop(prop::ITEMS)).unwrap_or("");
                let opts: Vec<&str> = list.split('\u{1F}').collect();
                Value::Str(opts[rng.below(opts.len())].to_owned())
            }
            _ => Value::Str(["", "a", "Hello", "42", "x y z"][rng.below(5)].to_owned()),
        },
        event::KEY_DOWN | event::KEY_UP => Value::Str(["ArrowUp", "ArrowDown", "Enter", "a"][rng.below(4)].to_owned()),
        event::POINTER_DOWN | event::POINTER_MOVE | event::POINTER_UP => {
            Value::List(vec![Value::Number(rng.below(400) as f64), Value::Number(rng.below(300) as f64)])
        }
        _ => Value::Null,
    }
}

fn apply(name: &str, tree: &mut Tree, commits: anyhow::Result<Vec<Vec<u8>>>) {
    for c in commits.unwrap_or_else(|e| panic!("{name}: the app stopped: {e:#}")) {
        let errors = tree.apply(&c).unwrap();
        assert!(errors.is_empty(), "{name}: op errors: {errors:?}");
    }
}

fn seed() -> Vec<u8> {
    plinth_protocol::init_arg::one(plinth_protocol::init_arg::RANDOM_SEED, &7u64.to_le_bytes())
}

#[test]
fn memory_after_init_and_events() {
    let _serial = serial();
    let runner = Runner::new().unwrap();
    let mut values = BTreeMap::new();
    for name in RUN_APPS {
        let art = build_example(name);
        let mut guest = runner.load(&art.component, Limits::default()).unwrap();
        let mut tree = Tree::new();
        apply(name, &mut tree, guest.init(&seed()));
        let init = guest.memory_bytes() as u64;
        // A fixed sequence of UI events: no timers and no clock, so the
        // result is the same in each run.
        let mut rng = Rng(0x5EED);
        for _ in 0..EVENTS {
            let ls = listeners(&tree);
            if ls.is_empty() {
                break;
            }
            let (id, ev, handler) = ls[rng.below(ls.len())];
            let value = value_for(&mut rng, &tree, id, ev);
            let mut w = Writer::new();
            w.event(&Event::Ui { handler, event: ev, value });
            apply(name, &mut tree, guest.on_event(w.as_bytes()));
            let _ = guest.take_logs();
            let _ = guest.take_errors();
        }
        let after = guest.memory_bytes() as u64;
        eprintln!("memory {name:<10} init {:>6} KiB, after {EVENTS} events {:>6} KiB, {} nodes", init >> 10, after >> 10, tree.len());
        values.insert(format!("{name}.init"), init);
        values.insert(format!("{name}.events"), after);
    }
    check("memory", "B", &values, |_, v| with_margin(v).div_ceil(PAGE) * PAGE);
}

/// The start time of each app: `Runner::load` (the engine compiles the core
/// and the app) plus `init` to the first commit. The compiler is not in it.
///
/// Noise: other builds on the same machine make one sample slow, never fast.
/// The test takes `START_ROUNDS` rounds that go through the apps in turn
/// (interleaved, so that a slow period touches all apps, not one), and uses
/// the median of the rounds. The budget is `START_FACTOR` times the median
/// at bless time: it catches a large regression (an app that builds its UI
/// two times, a core that compiles much slower), not a small one.
#[test]
fn start_time() {
    let _serial = serial();
    let runner = Runner::new().unwrap();
    let apps: Vec<(&str, Vec<u8>)> = RUN_APPS.iter().map(|n| (*n, build_example(n).component)).collect();
    // One load first, so that the first sample does no one-time work.
    drop(runner.load(&apps[0].1, Limits::default()).unwrap());
    let mut samples: BTreeMap<&str, Vec<(Duration, Duration)>> = BTreeMap::new();
    for _ in 0..START_ROUNDS {
        for (name, component) in &apps {
            let t0 = Instant::now();
            let mut guest = runner.load(component, Limits::default()).unwrap();
            let t1 = Instant::now();
            let commits = guest.init(&seed()).unwrap();
            let t2 = Instant::now();
            assert!(!commits.is_empty(), "{name}: no first commit");
            samples.entry(name).or_default().push((t1 - t0, t2 - t1));
        }
    }
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    let mut values = BTreeMap::new();
    eprintln!("start time ({} build, median of {START_ROUNDS} rounds):", if cfg!(debug_assertions) { "debug" } else { "release" });
    for (name, s) in &samples {
        let mut total: Vec<f64> = s.iter().map(|(l, i)| ms(*l + *i)).collect();
        let mut load: Vec<f64> = s.iter().map(|(l, _)| ms(*l)).collect();
        let mut init: Vec<f64> = s.iter().map(|(_, i)| ms(*i)).collect();
        for v in [&mut total, &mut load, &mut init] {
            v.sort_by(f64::total_cmp);
        }
        let m = START_ROUNDS / 2;
        eprintln!(
            "  {name:<10} total {:>8.1} ms (min {:>8.1}, max {:>8.1}), load {:>8.1} ms, init {:>7.1} ms",
            total[m],
            total[0],
            total[START_ROUNDS - 1],
            load[m],
            init[m]
        );
        values.insert(format!("{name}.load"), load[m].ceil() as u64);
        values.insert(format!("{name}.init"), init[m].ceil() as u64);
    }
    check("start_ms", "ms", &values, |k, v| {
        let floor = if k.ends_with(".init") { INIT_FLOOR_MS } else { LOAD_FLOOR_MS };
        (v as f64 * START_FACTOR).max(floor).ceil() as u64
    });
}
