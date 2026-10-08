//! Array `splice`/`fill`/`flat`, `Map.entries()` as a value, and dynamic
//! `Chart` data (HANDOFF.md §9 item 4). A separate file so that this work
//! does not collide with other branches that edit `lang.rs`.

use plinth_compiler::driver::{Frontend, MemFs, frontend};
use plinth_protocol::ControlKind;
use plinth_runner_wasmtime::{Limits, Runner};
use plinth_ui::tree::Tree;

const APP: &str = "\nexport default app({ screens: { home: { title: \"Home\", component: Home } } });\n";

fn render(f: &Frontend) -> String {
    f.diags.iter().map(|d| f.sources.render(d)).collect::<Vec<_>>().join("\n")
}

fn codes(main: &str) -> Vec<&'static str> {
    let fs = MemFs::default().with("app/main.tsx", main);
    let f = frontend(&fs);
    eprintln!("{}", render(&f));
    f.diags.iter().map(|d| d.code).collect()
}

/// Compiles and runs a program; returns the initial tree. `stress` turns on
/// the GC stress mode.
fn run_with(main: &str, stress: bool) -> Tree {
    let fs = MemFs::default().with("app/main.tsx", main);
    let (front, artifact) = plinth_compiler::compile(&fs).expect("compile");
    let artifact = artifact.unwrap_or_else(|| {
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        panic!("compile errors:\n{}", diags.join("\n"))
    });
    let runner = Runner::new().unwrap();
    let mut guest = runner.load(&artifact.component, Limits::default()).unwrap();
    let mut tree = Tree::new();
    let args = if stress { plinth_protocol::init_arg::one(plinth_protocol::init_arg::GC_STRESS, &[]) } else { Vec::new() };
    let commits = guest.init(&args).unwrap();
    for log in guest.take_logs() {
        eprintln!("guest: {log}");
    }
    for commit in commits {
        let errors = tree.apply(&commit).unwrap();
        assert!(errors.is_empty(), "op errors: {errors:?}");
    }
    tree
}

fn run(main: &str) -> Tree {
    run_with(main, false)
}

fn text_of(tree: &Tree, kind: ControlKind) -> String {
    let mut stack: Vec<_> = tree.screens().map(|(_, id)| id).collect();
    while let Some(id) = stack.pop() {
        let node = tree.get(id).unwrap();
        if node.kind == Some(kind) {
            if let Some(t) = &node.text {
                return t.clone();
            }
        }
        stack.extend(node.children.iter());
    }
    panic!("no {kind:?} found");
}

/// A program whose `Home` shows the string expression `out`, computed from
/// the statements in `body`.
fn show(body: &str, out: &str) -> String {
    format!(
        "import {{ app, Screen, Text }} from \"plinth:ui\";\nfunction s(a: number[]): string {{ return a.map((x) => \"\" + x).join(\",\"); }}\nfunction Home() {{\n{body}\n  return <Screen title=\"Home\"><Text>{{{out}}}</Text></Screen>;\n}}\n{APP}"
    )
}

fn shown(body: &str, out: &str) -> String {
    text_of(&run(&show(body, out)), ControlKind::Text)
}

// -- Array `splice` ----------------------------------------------------------

#[test]
fn splice_removes_and_inserts() {
    let out = shown("const a = [1, 2, 3, 4, 5]; const r = a.splice(1, 2, 9, 8, 7);", "s(a) + \"|\" + s(r)");
    assert_eq!(out, "1,9,8,7,4,5|2,3");
}

#[test]
fn splice_with_a_negative_start_and_no_count_removes_to_the_end() {
    let out = shown("const a = [1, 2, 3, 4, 5]; const r = a.splice(-2);", "s(a) + \"|\" + s(r)");
    assert_eq!(out, "1,2,3|4,5");
}

#[test]
fn splice_clamps_the_start_and_the_count() {
    let body = "const a = [1, 2, 3]; const r1 = a.splice(1, -4); const r2 = a.splice(10, 1, 4); const r3 = a.splice(-10, 2); const r4 = a.splice(0, 99);";
    let out = shown(body, "s(r1) + \"|\" + s(r2) + \"|\" + s(r3) + \"|\" + s(r4) + \"|\" + a.length");
    assert_eq!(out, "||1,2|3,4|0");
}

#[test]
fn splice_only_inserts_with_a_zero_count() {
    let out = shown("const a = [1, 4]; const r = a.splice(1, 0, 2, 3);", "s(a) + \"|\" + r.length");
    assert_eq!(out, "1,2,3,4|0");
}

#[test]
fn splice_evaluates_items_before_it_changes_the_array() {
    let out = shown("const a = [1, 2, 3]; a.splice(0, 3, a.length);", "s(a)");
    assert_eq!(out, "3");
}

#[test]
fn splice_on_strings_survives_gc_stress() {
    let main = show(
        "const a = [\"a\", \"b\", \"c\", \"d\"]; const r = a.splice(1, 2, \"x\" + a.length);",
        "a.join(\",\") + \"|\" + r.join(\",\")",
    );
    assert_eq!(text_of(&run_with(&main, true), ControlKind::Text), "a,x4,d|b,c");
}

#[test]
fn splice_without_arguments_is_rejected() {
    assert_eq!(codes(&show("const a = [1]; a.splice();", "\"\"")), ["PL3006"]);
}

// -- Array `fill` ------------------------------------------------------------

#[test]
fn fill_sets_every_element_and_returns_the_array() {
    let out = shown("const a = [0, 0, 0]; const b = a.fill(7);", "s(a) + \"|\" + (a === b)");
    assert_eq!(out, "7,7,7|true");
}

#[test]
fn fill_with_a_start_and_an_end() {
    let out = shown("const a = [0, 0, 0, 0, 0]; a.fill(1, 1, 3); a.fill(2, -1); a.fill(3, 2, -2); a.fill(4, 9);", "s(a)");
    assert_eq!(out, "0,1,3,0,2");
}

#[test]
fn fill_strings_and_structs() {
    let body = "const a = [\"a\", \"b\"]; a.fill(\"z\"); const p = { n: 1 }; const ps = [{ n: 0 }, { n: 0 }]; ps.fill(p, 1);";
    let out = shown(body, "a.join(\",\") + \"|\" + ps[0].n + ps[1].n");
    assert_eq!(out, "z,z|01");
}

#[test]
fn fill_without_a_value_is_rejected() {
    assert_eq!(codes(&show("const a = [1]; a.fill();", "\"\"")), ["PL3006"]);
}

// -- Array `flat` ------------------------------------------------------------

#[test]
fn flat_flattens_one_level() {
    let out = shown("const a = [[1, 2], [], [3]]; const b = a.flat(); b.push(4);", "s(b) + \"|\" + a.length");
    assert_eq!(out, "1,2,3,4|3");
}

#[test]
fn flat_of_strings_with_an_explicit_depth_of_one() {
    let main = show("const a = [[\"a\"], [\"b\", \"c\"]]; const b = a.flat(1);", "b.join(\",\")");
    assert_eq!(text_of(&run_with(&main, true), ControlKind::Text), "a,b,c");
}

#[test]
fn flat_on_a_flat_array_is_a_copy() {
    let out = shown("const a = [1, 2]; const b = a.flat(); b.push(3);", "s(a) + \"|\" + s(b)");
    assert_eq!(out, "1,2|1,2,3");
}

#[test]
fn flat_with_a_larger_depth_and_infinity() {
    let body = r#"const a = [[[1, 2], [3]], [], [[4], []]];
const two: number[] = a.flat(2);
const one: number[][] = a.flat(1);
const all: number[] = a.flat(Infinity);
const more: number[] = a.flat(7);
const none: number[][][] = a.flat(0);
none.push([]);
const deep = [[[["x"]]], [[["y", "z"]]]];
const d3: string[] = deep.flat(3);
const d2: string[][] = deep.flat(2);
const flatAlready: number[] = [5, 6].flat(Infinity);"#;
    let out = "s(two) + \"|\" + one.length + \"|\" + s(all) + \"|\" + s(more) + \"|\" + none.length + a.length + \"|\" + d3.join(\"\") + d2.length + \"|\" + s(flatAlready) + \"|\" + (Infinity > 1e308) + (NaN === NaN)";
    let main = show(body, out);
    assert_eq!(text_of(&run_with(&main, true), ControlKind::Text), "1,2,3,4|4|1,2,3,4|1,2,3,4|43|xyz2|5,6|truefalse");
}

#[test]
fn flat_with_a_computed_depth_is_rejected() {
    assert_eq!(codes(&show("const a = [[[1]]]; const n = 2; const b = a.flat(n);", "\"\"")), ["PL2000"]);
}

// -- `Map.entries()` as a value, and tuples -----------------------------------

const MAP: &str = "const m = new Map<string, number>(); m.set(\"a\", 1); m.set(\"b\", 2);";

#[test]
fn map_entries_as_an_array_of_pairs() {
    let body = format!("{MAP} const es = m.entries(); const out = es.map(([k, v]) => k + \"=\" + v).join(\",\");");
    assert_eq!(shown(&body, "out + \"|\" + es.length"), "a=1,b=2|2");
}

#[test]
fn map_entries_index_and_destructure() {
    let body = format!("{MAP} const es = m.entries(); const first = es[0]; const [k, v] = es[1];");
    assert_eq!(shown(&body, "first[0] + first[1] + \"|\" + k + v"), "a1|b2");
}

#[test]
fn map_entries_are_copies() {
    let body = format!("{MAP} const es = m.entries(); es[0][1] = 9; es.pop();");
    assert_eq!(shown(&body, "\"\" + (m.get(\"a\") ?? 0) + m.size + es.length"), "121");
}

#[test]
fn map_entries_spread_sort_and_for_of() {
    let body = format!(
        "{MAP} m.set(\"c\", 0); const es = [...m.entries()].sort((x, y) => x[1] - y[1]); let out = \"\"; for (const [k, v] of es) {{ out = out + k + v; }}"
    );
    assert_eq!(shown(&body, "out"), "c0a1b2");
}

#[test]
fn for_of_over_a_map_with_one_name_binds_a_pair() {
    let body = format!("{MAP} let out = \"\"; for (const e of m) {{ out = out + e[0] + e[1]; }}");
    assert_eq!(shown(&body, "out"), "a1b2");
}

#[test]
fn tuple_annotations_literals_and_json() {
    let body = "const p: [string, number] = [\"x\", 3]; const ps: [string, number][] = [p, [\"y\", 4]]; const j = JSON.stringify(ps);";
    let main = show(body, "j + \"|\" + ps[1][0]").replace("from \"plinth:ui\";", "from \"plinth:ui\";
import { JSON } from \"plinth:core\";");
    assert_eq!(text_of(&run(&main), ControlKind::Text), "[[\"x\",3],[\"y\",4]]|y");
}

#[test]
fn map_entries_survive_gc_stress() {
    let main = show(
        &format!("{MAP} const es = m.entries(); m.clear();"),
        "es.map((e) => e[0] + e[1]).join(\",\")",
    );
    assert_eq!(text_of(&run_with(&main, true), ControlKind::Text), "a1,b2");
}

#[test]
fn tuple_index_must_be_a_literal_in_range() {
    let body = format!("{MAP} const e = m.entries()[0]; const i = 1; const a = e[2]; const b = e[i];");
    assert_eq!(codes(&show(&body, "\"\"")), ["PL2011", "PL2011"]);
}

#[test]
fn tuple_literal_with_the_wrong_length_is_rejected() {
    assert_eq!(codes(&show("const p: [string, number] = [\"x\"];", "\"\"")), ["PL3001"]);
}


// -- Tuples with optional and rest elements ------------------------------------

#[test]
fn tuple_with_optional_elements() {
    let body = r#"const a: [number, string?] = [1];
const b: [number, string?] = [2, "x"];
const c: [number, string?, boolean?] = [3, null, true];
const n = (b[1] ?? "-") + (a[1] ?? "-");
const [x, y] = b;
let t: [number, string?] = [4];
t[1] = "set";
const lens = "" + a.length + b.length + c.length + t.length;"#;
    assert_eq!(shown(body, r#"n + "|" + x + (y ?? "") + "|" + lens + "|" + (t[1] ?? "")"#), "x-|2x|1232|set");
}

#[test]
fn tuple_with_a_rest_element() {
    let body = r#"const xs = [7, 8];
const a: [string, ...number[]] = ["a", 1, 2, 3];
const b: [string, ...number[]] = ["b"];
const c: [string, boolean?, ...number[]] = ["c", true, ...xs, 9];
let d: [string, ...number[]] = ["d", ...xs];
d[2] = 5;
const [h, f, g] = a;
const all: [...number[]] = [4, 5];
const out = a[0] + a[1] + a[3] + "|" + a.length + b.length + c.length + d.length + "|" + h + f + g + "|" + d[1] + d[2] + "|" + all.length;"#;
    assert_eq!(shown(body, "out"), "a13|4153|a12|75|2");
}

#[test]
fn tuple_with_optional_and_rest_elements_to_json() {
    let body = r#"const a: [string, ...number[]] = ["a", 1, 2];
const b: [string, ...number[]] = ["b"];
const c: [number, string?] = [1];
const j = JSON.stringify(a) + JSON.stringify(b) + JSON.stringify(c);"#;
    let main = show(body, "j").replace("from \"plinth:ui\";", "from \"plinth:ui\";\nimport { JSON } from \"plinth:core\";");
    assert_eq!(text_of(&run(&main), ControlKind::Text), r#"["a",1,2]["b"][1,null]"#);
}

#[test]
fn tuple_with_optional_and_rest_elements_diagnostics() {
    // Too few elements, too many, a spread in the fixed part.
    assert_eq!(codes(&show(r#"const a: [number, number, string?] = [1];"#, "\"\"")), ["PL3001"]);
    assert_eq!(codes(&show(r#"const a: [number, string?] = [1, "a", "b"];"#, "\"\"")), ["PL3001"]);
    assert_eq!(codes(&show(r#"const xs = [1]; const a: [number, ...number[]] = [...xs];"#, "\"\"")), ["PL3001"]);
    // The rest element is not last, a required element after an optional one,
    // a rest element that is not an array.
    assert_eq!(codes(&show(r#"let a: [...number[], string] = [""];"#, "\"\"")), ["PL2012"]);
    assert_eq!(codes(&show(r#"let a: [number?, string] = [1, ""];"#, "\"\"")), ["PL1000"]);
    assert_eq!(codes(&show(r#"let a: [string, ...[number, number]] | null = null;"#, "\"\"")), ["PL2012"]);
    // A computed index into a tuple with a rest element.
    assert_eq!(codes(&show(r#"const a: [string, ...number[]] = ["a"]; const i = 1; const v = a[i];"#, "\"\"")), ["PL2011"]);
}
#[test]
fn map_set_with_a_value_that_reads_the_same_new_key() {
    // Before the fix, `set` pushed the key and then evaluated the value, so
    // `get` found the key with no value and read out of bounds.
    let body = "const xs = [\"a\", \"b\", \"a\"]; const m = new Map<string, number>(); for (const t of xs) { m.set(t, (m.get(t) ?? 0) + 1); } let out = \"\"; for (const [k, v] of m) { out = out + k + v; }";
    assert_eq!(shown(body, "out"), "a2b1");
}

// -- `std/lib.d.ts` iteration types ------------------------------------------

/// `tsc` accepts `for…of` over a `Map`/`Set` only with these declarations
/// (CI runs `tsc` on `examples/budget`, which uses it). This guards against
/// an edit that removes them.
#[test]
fn lib_d_ts_declares_map_and_set_iterable() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../std/lib.d.ts");
    let lib = std::fs::read_to_string(path).unwrap();
    assert!(lib.contains("[Symbol.iterator](): Iterator<[K, V]>;"), "Map is not iterable");
    assert!(lib.contains("[Symbol.iterator](): Iterator<T>;"), "Set is not iterable");
    assert!(lib.contains("interface Iterable<T, TReturn = unknown, TNext = unknown>"), "the global Iterable needs three type parameters");
    assert!(lib.contains("interface Iterator<T, TReturn = unknown, TNext = unknown>"), "the global Iterator needs three type parameters");
}

// -- Dynamic `Chart` data --------------------------------------------------------

/// Compiles `main`, runs `init`, then presses the button labelled `press`
/// (if any). Returns the `Chart`'s `prop` string before and after.
fn chart_prop(main: &str, prop_id: u16, press: Option<&str>) -> (String, String) {
    use plinth_protocol::{Event, Value, Writer, event, prop};
    let fs = MemFs::default().with("app/main.tsx", main);
    let (front, artifact) = plinth_compiler::compile(&fs).expect("compile");
    let artifact = artifact.unwrap_or_else(|| {
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        panic!("compile errors:\n{}", diags.join("\n"))
    });
    let runner = Runner::new().unwrap();
    let mut guest = runner.load(&artifact.component, Limits::default()).unwrap();
    let mut tree = Tree::new();
    for commit in guest.init(&[]).unwrap() {
        assert!(tree.apply(&commit).unwrap().is_empty());
    }
    let find = |tree: &Tree, kind: ControlKind, label: Option<&str>| {
        let mut stack: Vec<_> = tree.screens().map(|(_, id)| id).collect();
        while let Some(id) = stack.pop() {
            let node = tree.get(id).unwrap();
            if node.kind == Some(kind) && label.is_none_or(|l| node.str_prop(prop::LABEL) == Some(l)) {
                return id;
            }
            stack.extend(node.children.iter());
        }
        panic!("no {kind:?}");
    };
    let chart = find(&tree, ControlKind::Chart, None);
    let before = tree.get(chart).unwrap().str_prop(prop_id).unwrap_or_default().to_owned();
    if let Some(label) = press {
        let button = find(&tree, ControlKind::Button, Some(label));
        let handler = tree.get(button).unwrap().handler(event::PRESS).unwrap();
        let mut w = Writer::new();
        w.event(&Event::Ui { handler, event: event::PRESS, value: Value::Null });
        for commit in guest.on_event(w.as_bytes()).unwrap() {
            assert!(tree.apply(&commit).unwrap().is_empty());
        }
    }
    let after = tree.get(chart).unwrap().str_prop(prop_id).unwrap_or_default().to_owned();
    (before, after)
}

fn chart_app(body: &str, chart_props: &str) -> String {
    format!(
        "import {{ app, Screen, Button, Chart, ChartPoint, signal, computed }} from \"plinth:ui\";\nfunction Home() {{\n{body}\n  return <Screen title=\"Home\"><Button label=\"Add\" onPress={{() => add()}} /><Chart label=\"c\" kind=\"bar\" {chart_props} /></Screen>;\n}}\n{APP}"
    )
}

#[test]
fn chart_data_from_a_mapped_signal_updates() {
    let body = "const xs = signal<number[]>([1, 2]);\n  const add = () => xs.set([...xs(), 3]);\n  const pts = computed(() => xs().map((x): ChartPoint => ({ label: \"p\" + x, value: x * 10 })));";
    let main = chart_app(body, "data={pts()}");
    let (before, after) = chart_prop(&main, plinth_protocol::prop::DATA, Some("Add"));
    assert_eq!(before, "p1\u{1}10\u{1f}p2\u{1}20");
    assert_eq!(after, "p1\u{1}10\u{1f}p2\u{1}20\u{1f}p3\u{1}30");
}

#[test]
fn chart_data_from_an_interface_with_more_fields_and_an_empty_array() {
    let body = "const rows = signal<Row[]>([]);\n  const add = () => rows.set([{ id: 1, label: \"a\", value: 1.5 }]);";
    let main = chart_app(body, "data={rows()}").replace("function Home", "interface Row { id: number; label: string; value: number }\nfunction Home");
    let (before, after) = chart_prop(&main, plinth_protocol::prop::DATA, Some("Add"));
    assert_eq!(before, "");
    assert_eq!(after, "a\u{1}1.5");
}

#[test]
fn chart_series_points_can_be_dynamic() {
    let body = "const a: ChartPoint[] = [{ label: \"x\", value: 1 }];\n  const add = () => {};";
    let main = chart_app(body, "data={a} series={[{ name: \"s\", points: a }, { name: \"t\", points: [{ label: \"y\", value: 2 }] }]}");
    let (before, _) = chart_prop(&main, plinth_protocol::prop::SERIES, None);
    assert_eq!(before, "s\u{1}x\u{1}1\u{1e}t\u{1}y\u{1}2");
}

#[test]
fn chart_data_of_the_wrong_type_is_rejected() {
    let main = chart_app("const xs = [1, 2];\n  const add = () => {};", "data={xs}");
    assert_eq!(codes(&main), ["PL3001"]);
    let main = chart_app("const xs = [{ label: 1, value: 2 }];\n  const add = () => {};", "data={xs}");
    assert_eq!(codes(&main), ["PL3001"]);
}

#[test]
fn chart_series_from_a_computed_map_updates() {
    let body = "const years = signal<number[]>([2025]);\n  const add = () => years.set([...years(), 2026]);\n  const series = computed(() => years().map((y): ChartSeriesDef => ({ name: \"\" + y, points: [{ label: \"q1\", value: y - 2000 }, { label: \"q2\", value: 1 }] })));";
    let main = chart_app(body, "data={[]} series={series()}").replace("ChartPoint, signal", "ChartPoint, ChartSeriesDef, signal");
    let (before, after) = chart_prop(&main, plinth_protocol::prop::SERIES, Some("Add"));
    assert_eq!(before, "2025\u{1}q1\u{1}25\u{1f}q2\u{1}1");
    assert_eq!(after, "2025\u{1}q1\u{1}25\u{1f}q2\u{1}1\u{1e}2026\u{1}q1\u{1}26\u{1f}q2\u{1}1");
}

#[test]
fn chart_series_from_a_variable_and_wrong_types() {
    let body = "const s: ChartSeriesDef[] = [];\n  const add = () => {};";
    let main = chart_app(body, "data={[]} series={s}").replace("ChartPoint, signal", "ChartPoint, ChartSeriesDef, signal");
    let (before, _) = chart_prop(&main, plinth_protocol::prop::SERIES, None);
    assert_eq!(before, "");
    let main = chart_app("const s = [{ name: \"a\", points: [1] }];\n  const add = () => {};", "data={[]} series={s}");
    assert_eq!(codes(&main), ["PL3001"]);
}

// -- Speed of `Map`, `Set`, `sort` and `join` (docs/GAPS.md, engine step 3) --
//
// Before the fix, `Map`/`Set` searched the keys one by one and `sort` was an
// insertion sort: these sizes took far more than the 2 s call limit of
// `Limits::default()`, so the run stopped with an error.

#[test]
fn map_with_20000_string_keys_is_fast() {
    let body = r#"
  const m = new Map<string, number>();
  for (let i = 0; i < 20000; i++) { m.set("key" + i, i); }
  let sum = 0;
  for (let i = 0; i < 20000; i++) { sum += m.get("key" + i) ?? 0; if (!m.has("key" + i)) { sum = -1; } }
  for (let i = 0; i < 20000; i += 2) { m.delete("key" + i); }
  let first = "";
  for (const [k, v] of m) { if (first === "") { first = k; } }
  const left = m.size;
  for (let i = 1; i < 20000; i += 2) { m.delete("key" + i); }"#;
    assert_eq!(shown(body, r#"sum + "|" + left + "|" + first + "|" + m.size + "|" + m.has("key1")"#), "199990000|10000|key1|0|false");
}

#[test]
fn set_with_20000_number_keys_is_fast() {
    let body = r#"
  const s = new Set<number>();
  for (let i = 0; i < 20000; i++) { s.add(i * 0.5); s.add(i * 0.5); }
  let n = 0;
  for (let i = 0; i < 20000; i++) { if (s.has(i * 0.5)) { n++; } }
  for (let i = 0; i < 20000; i += 4) { s.delete(i * 0.5); }"#;
    assert_eq!(shown(body, r#"n + "|" + s.size + "|" + s.has(0) + s.has(0.5) + s.has(-0)"#), "20000|15000|falsetruefalse");
}

#[test]
fn map_keeps_insertion_order_after_delete_and_set_again() {
    let body = r#"
  const m = new Map<string, number>();
  m.set("a", 1); m.set("b", 2); m.set("c", 3); m.set("d", 4);
  m.delete("b");
  m.set("a", 10);
  m.set("b", 20);
  let out = "";
  for (const [k, v] of m) { out += k + v + ","; }
  let fe = "";
  m.forEach((v, k) => { fe += k; });
  const ks = m.keys().join("");
  const vs = m.values().map((x) => "" + x).join(",");
  const es = m.entries().map((e) => e[0]).join("");
  const js = JSON.stringify(m);"#;
    let main = show(body, r#"out + "|" + fe + "|" + ks + "|" + vs + "|" + es + "|" + js + "|" + m.size"#)
        .replace("from \"plinth:ui\";", "from \"plinth:ui\";\nimport { JSON } from \"plinth:core\";");
    assert_eq!(text_of(&run(&main), ControlKind::Text), r#"a10,c3,d4,b20,|acdb|acdb|10,3,4,20|acdb|{"a":10,"c":3,"d":4,"b":20}|4"#);
}

#[test]
fn map_delete_during_for_of_skips_the_deleted_entry() {
    let body = r#"
  const m = new Map<string, number>();
  m.set("a", 1); m.set("b", 2); m.set("c", 3);
  let out = "";
  for (const [k, v] of m) { out += k; if (k === "a") { m.delete("b"); m.set("d", 4); } }
  const s = new Set<string>(); s.add("x"); s.add("y"); s.add("z");
  let so = "";
  s.forEach((v) => { so += v; if (v === "x") { s.delete("y"); } });
  for (const v of s) { so += v; }
  s.clear(); s.add("y");
  m.clear(); m.set("q", 1);"#;
    assert_eq!(shown(body, r#"out + "|" + so + "|" + s.size + s.has("y") + s.has("x") + "|" + m.size + m.has("a") + (m.get("q") ?? 0)"#), "acd|xzxz|1truefalse|1false1");
}

#[test]
fn map_with_int_bool_and_enum_keys() {
    let body = r#"
  const m = new Map<boolean, string>();
  m.set(true, "t"); m.set(false, "f"); m.set(true, "T");
  const n = new Map<number, string>();
  n.set(1, "one"); n.set(2, "two"); n.delete(1); n.set(1, "uno");
  let o = "";
  for (const [k, v] of n) { o += k + v; }"#;
    assert_eq!(shown(body, r#"(m.get(true) ?? "") + (m.get(false) ?? "") + m.size + "|" + o"#), "Tf2|2two1uno");
}

#[test]
fn map_survives_gc_stress_with_holes() {
    let main = show(
        r#"
  const m = new Map<string, string>();
  for (let i = 0; i < 300; i++) { m.set("k" + i, "v" + i); }
  for (let i = 0; i < 300; i += 3) { m.delete("k" + i); }
  for (let i = 0; i < 300; i++) { m.set("n" + i, "w" + i); }
  let n = 0;
  for (const [k, v] of m) { n += v.length; }"#,
        r#""" + n + "|" + m.size + "|" + (m.get("k299") ?? "")"#,
    );
    assert_eq!(text_of(&run_with(&main, true), ControlKind::Text), "1818|500|v299");
}

#[test]
fn sort_is_stable_and_uses_the_comparator() {
    let body = r#"
  const ps = [{ n: "a", k: 2 }, { n: "b", k: 1 }, { n: "c", k: 2 }, { n: "d", k: 1 }, { n: "e", k: 0 }, { n: "f", k: 2 }, { n: "g", k: 1 }, { n: "h", k: 0 }, { n: "i", k: 2 }, { n: "j", k: 1 }, { n: "k", k: 0 }, { n: "l", k: 2 }];
  ps.sort((x, y) => x.k - y.k);
  const desc = [3, 1, 2, 5, 4].sort((x, y) => y - x);
  const def = [10, 9, 1].sort();
  const strs = ["b", "a", "C", "c", "B"].sort();
  const one = [1].sort((x, y) => x - y);
  const none: number[] = [];
  none.sort((x, y) => x - y);
  const bools = [true, false, true].sort();"#;
    assert_eq!(
        shown(body, r#"ps.map((p) => p.n).join("") + "|" + s(desc) + "|" + s(def) + "|" + strs.join("") + "|" + s(one) + "|" + none.length + "|" + bools.join(",")"#),
        "ehkbdgjacfil|5,4,3,2,1|1,10,9|BCabc|1|0|false,true,true"
    );
}

#[test]
fn sort_of_20000_numbers_is_fast() {
    let body = r#"
  const a: number[] = [];
  let x = 12345;
  for (let i = 0; i < 20000; i++) { x = (x * 1103515245 + 12345) % 2147483648; a.push(x % 1000); }
  const r = a.sort((p, q) => p - q);
  let ok = r === a;
  for (let i = 1; i < a.length; i++) { if (a[i - 1] > a[i]) { ok = false; } }
  const d: number[] = [];
  for (let i = 0; i < 20000; i++) { d.push(20000 - i); }
  d.sort();"#;
    assert_eq!(shown(body, r#"ok + "|" + a.length + "|" + d[0] + "," + d[1] + "," + d[19999]"#), "true|20000|1,10,9999");
}

#[test]
fn join_of_100000_numbers_does_not_run_out_of_memory() {
    let body = r#"
  const a: number[] = [];
  for (let i = 0; i < 100000; i++) { a.push(i); }
  const t = a.join(",");
  const b = [true, false].join("-");"#;
    assert_eq!(shown(body, r#"t.length + "|" + t.slice(0, 8) + "|" + b + "|" + [1.5, 2].join()"#), "588889|0,1,2,3,|true-false|1.5,2");
}

#[test]
fn map_used_as_an_lru_keeps_its_size_and_order() {
    // Delete and set again moves a key to the end; the holes go away when
    // the table grows, so 50,000 moves do not make the arrays grow.
    let body = r#"
  const m = new Map<string, number>();
  for (let i = 0; i < 100; i++) { m.set("k" + i, i); }
  for (let i = 0; i < 50000; i++) { const k = "k" + (i % 100); const v = m.get(k) ?? 0; m.delete(k); m.set(k, v + 1); }
  m.delete("k3"); m.set("k3", 0);
  const ks = m.keys();"#;
    assert_eq!(shown(body, r#"m.size + "|" + ks[0] + "|" + ks[98] + "|" + ks[99] + "|" + (m.get("k0") ?? 0)"#), "100|k0|k99|k3|500");
}

// -- Iteration while the map changes (each live entry one time, new entries
// visited, deleted ones not), also when the table grows during the loop.

/// A map `m` of `k0`..`k99` without the multiples of 3 (66 live entries,
/// 34 holes), and `seen`, a count of visits for each key.
const HOLED: &str = r#"
  const m = new Map<string, number>();
  for (let i = 0; i < 100; i++) { m.set("k" + i, i); }
  for (let i = 0; i < 100; i += 3) { m.delete("k" + i); }
  const seen = new Map<string, number>();
  let most = 0;
  const visit = (k: string) => { const c = (seen.get(k) ?? 0) + 1; seen.set(k, c); if (c > most) { most = c; } };"#;

#[test]
fn for_of_that_adds_many_keys_visits_each_entry_one_time() {
    let body = format!(
        "{HOLED}\n  for (const [k, v] of m) {{ visit(k); if (k.startsWith(\"k\")) {{ for (let j = 0; j < 10; j++) {{ m.set(\"n\" + k + \"_\" + j, j); }} }} }}"
    );
    assert_eq!(
        shown(&body, r#"seen.size + "|" + most + "|" + seen.has("k0") + seen.has("nk1_9") + "|" + m.size + "|" + m.keys()[0] + "," + m.keys()[66]"#),
        "726|1|falsetrue|726|k1,nk1_0"
    );
}

#[test]
fn for_each_that_adds_many_keys_visits_each_entry_one_time() {
    let body = format!(
        "{HOLED}\n  m.forEach((v, k) => {{ visit(k); if (k.startsWith(\"k\")) {{ for (let j = 0; j < 10; j++) {{ m.set(\"n\" + k + \"_\" + j, j); }} m.delete(\"k\" + (v + 1)); }} }});"
    );
    // Each visited `k<i>` deletes `k<i+1>` before the loop gets to it.
    let out = shown(&body, r#"seen.size + "|" + most + "|" + seen.has("k1") + seen.has("k2") + seen.has("k4") + "|" + m.size"#);
    assert_eq!(out, "363|1|truefalsetrue|363");
}

#[test]
fn keys_and_json_inside_for_each_of_a_map_with_holes() {
    let body = r#"
  const m = new Map<string, number>();
  m.set("a", 1); m.set("b", 2); m.set("c", 3); m.set("d", 4);
  m.delete("b");
  let out = "";
  m.forEach((v, k) => { out += k + "=" + m.keys().join("") + JSON.stringify(m) + ";"; if (k === "a") { m.delete("c"); m.set("e", 5); } });"#;
    let main = show(body, "out").replace("from \"plinth:ui\";", "from \"plinth:ui\";\nimport { JSON } from \"plinth:core\";");
    assert_eq!(text_of(&run(&main), ControlKind::Text), r#"a=acd{"a":1,"c":3,"d":4};d=ade{"a":1,"d":4,"e":5};e=ade{"a":1,"d":4,"e":5};"#);
}

#[test]
fn nested_loops_over_the_same_map() {
    let body = r#"
  const m = new Map<string, number>();
  m.set("x", 0); m.set("a", 1); m.set("b", 2); m.set("c", 3);
  m.delete("x");
  let outer = "";
  let pairs = 0;
  for (const [k1, v1] of m) {
    outer += k1;
    for (const [k2, v2] of m) {
      pairs++;
      if (k1 === "a" && k2 === "a") { for (let j = 0; j < 20; j++) { m.set("n" + j, j); } }
    }
  }
  m.forEach((v, k) => { m.forEach((w, q) => { if (q === "b") { m.delete("b"); } }); });
  let after = "";
  for (const k of m.keys()) { after += k; }"#;
    assert_eq!(shown(body, r#"outer.slice(0, 5) + "|" + outer.length + "|" + pairs + "|" + after.slice(0, 5) + "|" + m.size"#), "abcn0|53|529|acn0n|22");
}

#[test]
fn break_return_and_throw_out_of_a_loop_then_more_changes() {
    let body = r#"
  const m = new Map<string, number>();
  for (let i = 0; i < 10; i++) { m.set("k" + i, i); }
  const firstOver = (lim: number): string => { for (const [k, v] of m) { if (v > lim) { return k; } } return ""; };
  let r = firstOver(4);
  for (const k of m.keys()) { if (k === "k2") { break; } }
  for (const [k, v] of m) { if (v === 5) { break; } }
  m.forEach((v, k) => { if (v === 1) { return; } });
  try { for (const [k, v] of m) { if (v === 3) { throw new Error("stop"); } } } catch (e) { r += "!"; }
  for (const [k, v] of m) { for (const [q, w] of m) { if (w === 2) { break; } } if (v === 6) { continue; } }
  for (let i = 0; i < 2000; i++) { const k = "k" + (i % 10); const v = m.get(k) ?? 0; m.delete(k); m.set(k, v + 1); }
  m.delete("k4"); m.set("k4", 0); m.delete("k7");
  let out = "";
  for (const [k, v] of m) { out += k; }"#;
    assert_eq!(shown(body, r#"r + "|" + out + "|" + m.size + "|" + (m.get("k0") ?? 0)"#), "k5!|k0k1k2k3k5k6k8k9k4|9|200");
}
