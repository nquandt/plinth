//! Compiler language-gap tests (HANDOFF.md §9 item 3). Separate from
//! `e2e.rs` so this agent does not collide with others editing that file.
//!
//! Diagnostic tests use an in-memory file system; behavior tests compile a
//! small Plinth TS program and run it in wasmtime, like `e2e.rs` does for
//! the example apps.

use plinth_compiler::driver::{Frontend, MemFs, frontend};
use plinth_protocol::{ControlKind, Event, Value, Writer, event, prop};
use plinth_runner_wasmtime::{Limits, Runner};
use plinth_ui::tree::Tree;

const APP: &str = "\nexport default app({ screens: { home: { title: \"Home\", component: Home } } });\n";

fn with_app(body: &str) -> String {
    format!("import {{ app, Screen, Text, signal, computed, effect }} from \"plinth:ui\";\n{body}\nfunction Home() {{ return <Screen title=\"Home\" />; }}{APP}")
}

fn render(f: &Frontend) -> String {
    f.diags.iter().map(|d| f.sources.render(d)).collect::<Vec<_>>().join("\n")
}

fn codes(main: &str) -> Vec<&'static str> {
    let fs = MemFs::default().with("app/main.tsx", main);
    let f = frontend(&fs);
    eprintln!("{}", render(&f));
    f.diags.iter().map(|d| d.code).collect()
}

/// Compiles a program and runs it in wasmtime; panics with the diagnostics
/// on a compile error. Returns the initial semantic tree.
fn run(main: &str) -> Tree {
    let fs = MemFs::default().with("app/main.tsx", main);
    let (front, artifact) = plinth_compiler::compile(&fs).expect("compile");
    let artifact = artifact.unwrap_or_else(|| {
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        panic!("compile errors:\n{}", diags.join("\n"))
    });
    let runner = Runner::new().unwrap();
    let mut guest = runner.load(&artifact.component, Limits::default()).unwrap();
    let mut tree = Tree::new();
    let commits = guest.init(&[]).unwrap();
    for log in guest.take_logs() {
        eprintln!("guest: {log}");
    }
    for commit in commits {
        let errors = tree.apply(&commit).unwrap();
        assert!(errors.is_empty(), "op errors: {errors:?}");
    }
    tree
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

// -- 1. Reactivity lint: a signal/computed read outside JSX, `computed` or
// `effect`, but inside a component body, is not reactive (HANDOFF.md §6). --

#[test]
fn signal_read_in_component_body_warns() {
    let main = with_app("const s = signal(1); const x = s();");
    assert_eq!(codes(&main), ["PL2020"]);
}

#[test]
fn computed_read_in_component_body_warns() {
    let main = with_app("const s = signal(1); const c = computed(() => s()); const x = c();");
    assert_eq!(codes(&main), ["PL2020"]);
}

#[test]
fn signal_read_in_jsx_child_is_fine() {
    let main = with_app("const s = signal(1);").replace(
        "function Home() { return <Screen title=\"Home\" />; }",
        "function Home() { return <Screen title=\"Home\"><Text>{s()}</Text></Screen>; }",
    );
    assert_eq!(codes(&main), Vec::<&str>::new());
}

#[test]
fn signal_read_in_jsx_prop_is_fine() {
    let main = r#"import { app, Screen, Text, signal } from "plinth:ui";
const s = signal("hi");
function Home() { return <Screen title={s()} />; }
"#
    .to_string()
        + APP;
    assert_eq!(codes(&main), Vec::<&str>::new());
}

#[test]
fn signal_read_in_computed_is_fine() {
    let main = with_app("const s = signal(1); const c = computed(() => s() + 1);");
    assert_eq!(codes(&main), Vec::<&str>::new());
}

#[test]
fn signal_read_in_effect_is_fine() {
    let main = with_app("const s = signal(1); effect(() => { const x = s(); });");
    assert_eq!(codes(&main), Vec::<&str>::new());
}

#[test]
fn signal_read_in_event_handler_is_fine() {
    let main = r#"import { app, Screen, Button, signal } from "plinth:ui";
const s = signal(1);
function Home() { return <Screen title="Home"><Button label="go" onPress={() => { const x = s(); }} /></Screen>; }
"#
    .to_string()
        + APP;
    assert_eq!(codes(&main), Vec::<&str>::new());
}

// -- 2. Discriminated unions + narrowing -------------------------------------

const SHAPE: &str = r#"
interface Circle { kind: "circle"; radius: number; }
interface Square { kind: "square"; side: number; }
type Shape = Circle | Square;
function area(s: Shape): number {
  if (s.kind === "circle") {
    return s.radius * s.radius * 3;
  } else {
    return s.side * s.side;
  }
}
"#;

#[test]
fn discriminant_narrowing_compiles_and_runs() {
    let main = format!(
        "import {{ app, Screen, Text }} from \"plinth:ui\";\n{SHAPE}\nfunction Home() {{ return <Screen title=\"Home\"><Text>{{area({{ kind: \"circle\", radius: 2 }})}}</Text></Screen>; }}{APP}"
    );
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "12");
}

#[test]
fn discriminant_narrowing_other_member() {
    let main = format!(
        "import {{ app, Screen, Text }} from \"plinth:ui\";\n{SHAPE}\nfunction Home() {{ return <Screen title=\"Home\"><Text>{{area({{ kind: \"square\", side: 3 }})}}</Text></Screen>; }}{APP}"
    );
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "9");
}

#[test]
fn union_field_without_narrowing_is_rejected() {
    let main = format!(
        "import {{ app, Screen }} from \"plinth:ui\";\n{SHAPE}\nfunction bad(s: Shape): number {{ return s.radius; }}\nfunction Home() {{ return <Screen title=\"Home\" />; }}{APP}"
    );
    assert_eq!(codes(&main), ["PL3004"]);
}

#[test]
fn union_member_mismatch_is_rejected() {
    // `number` is not a supported union member type yet.
    let main = with_app("function f(x: number | { a: number }): number { return 1; }");
    assert_eq!(codes(&main), ["PL2012"]);
}

#[test]
fn typeof_narrows_string_or_object_union() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
interface Circle { kind: "circle"; radius: number; }
function describe(x: string | Circle): string {
  if (typeof x === "string") {
    return x;
  } else {
    return "circle " + x.radius;
  }
}
function Home() {
  return <Screen title="Home"><Text>{describe("hi")}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "hi");
}

#[test]
fn typeof_narrows_to_object_branch() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
interface Circle { kind: "circle"; radius: number; }
function describe(x: string | Circle): string {
  if (typeof x === "string") {
    return x;
  } else {
    return "circle " + x.radius;
  }
}
function Home() {
  return <Screen title="Home"><Text>{describe({ kind: "circle", radius: 5 })}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "circle 5");
}

#[test]
fn bare_typeof_outside_comparison_is_rejected() {
    let main = with_app("function f(x: number): string { return typeof x; }");
    assert_eq!(codes(&main), ["PL2012"]);
}

// -- 3. Generic functions (monomorphized) ------------------------------------

#[test]
fn generic_identity_monomorphizes_per_type() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function identity<T>(x: T): T { return x; }
function Home() {
  const n = identity(1);
  const s = identity("hi");
  return <Screen title="Home"><Text>{s + "-" + (n + 1)}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "hi-2");
}

#[test]
fn generic_array_helper_infers_from_argument() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function first<T>(xs: T[]): T { return xs[0]; }
function Home() {
  const xs: number[] = [5, 6, 7];
  return <Screen title="Home"><Text>{first(xs)}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "5");
}

#[test]
fn generic_call_with_explicit_type_argument() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function identity<T>(x: T): T { return x; }
function Home() {
  return <Screen title="Home"><Text>{identity<string>("hey")}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "hey");
}

#[test]
fn generic_type_param_cannot_be_inferred() {
    // `T` only appears in the return type: nothing to infer it from.
    let main = with_app("function make<T>(): T[] { return []; }\nconst xs = make();");
    assert_eq!(codes(&main), ["PL3007"]);
}

#[test]
fn generic_function_used_as_a_value_is_rejected() {
    let main = with_app("function identity<T>(x: T): T { return x; }\nconst f = identity;");
    assert_eq!(codes(&main), ["PL2015"]);
}

#[test]
fn generic_call_reuses_the_same_instantiation() {
    // Calling with the same concrete type twice must still just work (no
    // duplicate-definition or redeclaration errors from monomorphizing).
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function identity<T>(x: T): T { return x; }
function Home() {
  const a = identity(1);
  const b = identity(2);
  return <Screen title="Home"><Text>{a + b}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "3");
}

// -- 4. `int` from plinth:core ------------------------------------------------

#[test]
fn int_arithmetic_stays_int_and_displays() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
import { int } from "plinth:core";
function Home() {
  const a: int = 7;
  const b = int(3);
  const c = a + b * int(2) - b;
  return <Screen title="Home"><Text>{c}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    // 7 + 3*2 - 3 = 10
    assert_eq!(text_of(&tree, ControlKind::Text), "10");
}

#[test]
fn int_widens_to_number_implicitly() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
import { int } from "plinth:core";
function Home() {
  const a: int = 7;
  const n: number = a;
  return <Screen title="Home"><Text>{n / 2}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "3.5");
}

#[test]
fn number_needs_explicit_int_conversion() {
    let main = with_app("const n: number = 5; const x: int = n;");
    assert_eq!(codes(&main), ["PL3001"]);
}

#[test]
fn int_div_by_int_stays_int_truncated() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
import { int } from "plinth:core";
function Home() {
  const a = int(7);
  const b = int(2);
  return <Screen title="Home"><Text>{a / b}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    // Integer division truncates: 7 / 2 = 3, unlike `number` division (3.5).
    assert_eq!(text_of(&tree, ControlKind::Text), "3");
}

// -- 5. Map / Set -------------------------------------------------------------

#[test]
fn map_set_get_has_delete_and_size() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const m = new Map<string, number>();
  m.set("a", 1);
  m.set("b", 2);
  m.set("a", 10);
  const hasA = m.has("a");
  const hasC = m.has("c");
  const a = m.get("a");
  const missing = m.get("z");
  const deletedB = m.delete("b");
  const deletedAgain = m.delete("b");
  const size = m.size;
  const av = a === null ? -1 : a;
  const mv = missing === null ? -1 : missing;
  const out = "" + hasA + "," + hasC + "," + av + "," + mv + "," + deletedB + "," + deletedAgain + "," + size;
  return <Screen title="Home"><Text>{out}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    // a was overwritten to 10; b deleted once; c and z never existed.
    assert_eq!(text_of(&tree, ControlKind::Text), "true,false,10,-1,true,false,1");
}

#[test]
fn map_clear_empties_it() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const m = new Map<string, number>();
  m.set("a", 1);
  m.set("b", 2);
  m.clear();
  return <Screen title="Home"><Text>{m.size}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "0");
}

#[test]
fn set_add_has_delete_and_size() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const s = new Set<number>();
  s.add(1);
  s.add(2);
  s.add(1);
  const has1 = s.has(1);
  const has9 = s.has(9);
  const deleted = s.delete(1);
  const deletedAgain = s.delete(1);
  const out = "" + has1 + "," + has9 + "," + deleted + "," + deletedAgain + "," + s.size;
  return <Screen title="Home"><Text>{out}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    // Adding 1 twice is a no-op; after deleting 1 once, only 2 remains.
    assert_eq!(text_of(&tree, ControlKind::Text), "true,false,true,false,1");
}

#[test]
fn map_key_must_be_a_supported_kind() {
    let main = with_app("const m = new Map<boolean[], number>();");
    assert_eq!(codes(&main), ["PL2012"]);
}

#[test]
fn map_new_without_type_args_or_context_cannot_infer() {
    let main = with_app("const m = new Map();");
    assert_eq!(codes(&main), ["PL3007"]);
}

#[test]
fn map_infers_type_args_from_declared_variable_type() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
import { int } from "plinth:core";
function Home() {
  const m: Map<string, int> = new Map();
  return <Screen title="Home"><Text>{m.size}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    // int is not a supported Map value type, but this still exercises
    // inference-from-context before that check; use number instead.
    let main = main.replace("Map<string, int>", "Map<string, number>");
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "0");
}

// -- 5b. `for…of` over `Map` / `Set` (HANDOFF.md item 1) ---------------------

#[test]
fn set_for_of_bare_visits_values_in_insertion_order() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const s = new Set<number>();
  s.add(1);
  s.add(2);
  s.add(3);
  let out = "";
  for (const v of s) { out = out + v + ","; }
  return <Screen title="Home"><Text>{out}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "1,2,3,");
}

#[test]
fn map_for_of_bare_destructures_key_value_pairs() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const m = new Map<string, number>();
  m.set("a", 1);
  m.set("b", 2);
  let out = "";
  for (const [k, v] of m) { out = out + k + "=" + v + ";"; }
  return <Screen title="Home"><Text>{out}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "a=1;b=2;");
}

#[test]
fn map_keys_values_and_entries_iterate_in_insertion_order() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const m = new Map<string, number>();
  m.set("a", 1);
  m.set("b", 2);
  let ks = "";
  for (const k of m.keys()) { ks = ks + k; }
  let vs = "";
  for (const v of m.values()) { vs = vs + v; }
  let es = "";
  for (const [k, v] of m.entries()) { es = es + k + v; }
  const out = ks + "," + vs + "," + es;
  return <Screen title="Home"><Text>{out}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "ab,12,a1b2");
}

#[test]
fn set_keys_is_the_same_as_a_bare_for_of() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const s = new Set<number>();
  s.add(5);
  s.add(6);
  let out = "";
  for (const v of s.keys()) { out = out + v; }
  return <Screen title="Home"><Text>{out}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "56");
}

#[test]
fn map_delete_keeps_insertion_order_of_the_rest() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const m = new Map<string, number>();
  m.set("a", 1);
  m.set("b", 2);
  m.set("c", 3);
  m.delete("b");
  let out = "";
  for (const [k, v] of m) { out = out + k + v; }
  return <Screen title="Home"><Text>{out}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    // Before this fix, `delete` swapped with the last entry instead of
    // shifting, so this would have come out as "a1c3" -> actually "c3a1"
    // (c moved into b's slot). Shifting keeps a, c in their original order.
    assert_eq!(text_of(&tree, ControlKind::Text), "a1c3");
}

#[test]
fn set_delete_keeps_insertion_order_of_the_rest() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const s = new Set<number>();
  s.add(1);
  s.add(2);
  s.add(3);
  s.add(4);
  s.delete(2);
  let out = "";
  for (const v of s) { out = out + v + ","; }
  return <Screen title="Home"><Text>{out}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "1,3,4,");
}

#[test]
fn set_for_of_rejects_a_key_value_pattern() {
    let main = with_app("const s = new Set<number>(); for (const [k, v] of s) {}");
    assert_eq!(codes(&main), ["PL3001"]);
}

#[test]
fn map_for_of_needs_a_key_value_pattern() {
    // `for (const e of m)` is valid (`e` is a `[K, V]` pair); an object
    // pattern is not.
    let main = with_app("const m = new Map<string, number>(); for (const { x } of m) {}");
    assert_eq!(codes(&main), ["PL3001"]);
}

#[test]
fn set_entries_is_not_supported() {
    let main = with_app("const s = new Set<number>(); for (const x of s.entries()) {}");
    assert_eq!(codes(&main), ["PL3004"]);
}

#[test]
fn map_foreach_visits_every_entry_in_insertion_order() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const m = new Map<string, number>();
  m.set("a", 1);
  m.set("b", 2);
  m.set("c", 3);
  let out = "";
  m.forEach((v, k) => { out = out + k + "=" + v + ","; });
  return <Screen title="Home"><Text>{out}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "a=1,b=2,c=3,");
}

#[test]
fn set_foreach_visits_every_value() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const s = new Set<number>();
  s.add(1);
  s.add(2);
  s.add(3);
  let out = "";
  s.forEach((v) => { out = out + v + ","; });
  return <Screen title="Home"><Text>{out}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "1,2,3,");
}

#[test]
fn map_keys_and_values_as_arrays() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const m = new Map<string, number>();
  m.set("a", 1);
  m.set("b", 2);
  const ks = [...m.keys()];
  const vs = m.values();
  const sum = vs.reduce((acc, v) => acc + v, 0);
  return <Screen title="Home"><Text>{ks.join(",") + "|" + sum}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "a,b|3");
}

#[test]
fn set_keys_and_values_as_arrays() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const s = new Set<number>();
  s.add(1);
  s.add(2);
  const ks = [...s.keys()];
  const sum = ks.reduce((acc, v) => acc + v, 0);
  return <Screen title="Home"><Text>{"" + sum}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "3");
}

// -- Array methods: concat/reduce (gap batch 3 item 3) -----------------------

#[test]
fn array_concat_joins_arrays() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const a = ["a", "b"];
  const b = ["c", "d"];
  const c = a.concat(b, ["e"]);
  return <Screen title="Home"><Text>{c.join(",")}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "a,b,c,d,e");
}

#[test]
fn array_concat_on_an_empty_array() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const a: number[] = [];
  const b: number[] = [];
  const c = a.concat(b);
  return <Screen title="Home"><Text>{"" + c.length}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "0");
}

#[test]
fn array_reduce_sums_with_an_initial_value() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const a = [1, 2, 3, 4];
  const sum = a.reduce((acc, v) => acc + v, 0);
  return <Screen title="Home"><Text>{"" + sum}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "10");
}

#[test]
fn array_reduce_on_an_empty_array_returns_the_initial_value() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const a: number[] = [];
  const sum = a.reduce((acc, v) => acc + v, 42);
  return <Screen title="Home"><Text>{"" + sum}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "42");
}

#[test]
fn array_reduce_with_index_builds_a_string() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const a = ["x", "y", "z"];
  const out = a.reduce((acc, v, i) => acc + i + v, "");
  return <Screen title="Home"><Text>{out}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "0x1y2z");
}

#[test]
fn array_reduce_without_an_initial_value_is_rejected() {
    let main = with_app("const a = [1, 2, 3]; const sum = a.reduce((acc, v) => acc + v);");
    assert_eq!(codes(&main), ["PL3006"]);
}

// -- Array methods: sort (gap "Larger items" #1) -----------------------------

#[test]
fn array_sort_numbers_with_a_comparator() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const a = [5, 3, 1, 4, 2];
  const b = a.sort((x, y) => x - y);
  const out = b.reduce((acc, v) => acc + v, "");
  return <Screen title="Home"><Text>{out}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "12345");
}

#[test]
fn array_sort_returns_the_same_array_mutated_in_place() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const a = [3, 1, 2];
  a.sort((x, y) => x - y);
  const out = a.reduce((acc, v) => acc + v, "");
  return <Screen title="Home"><Text>{out}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "123");
}

#[test]
fn array_sort_numbers_without_a_comparator_compares_as_strings_and_warns() {
    // JS default sort: string comparison, so 10 sorts before 2.
    let main = with_app("const a = [10, 2, 1]; const b = a.sort();");
    assert_eq!(codes(&main), ["PL2025"]);

    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const a = [10, 2, 1];
  const b = a.sort();
  const out = b.reduce((acc, v) => acc + v + ",", "");
  return <Screen title="Home"><Text>{out}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "1,10,2,");
}

#[test]
fn array_sort_strings_without_a_comparator() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const a = ["banana", "apple", "cherry"];
  const b = a.sort();
  return <Screen title="Home"><Text>{b.join(",")}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "apple,banana,cherry");
}

#[test]
fn array_sort_structs_by_a_key() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const a: { k: number; name: string }[] = [
    { k: 3, name: "c" },
    { k: 1, name: "a" },
    { k: 2, name: "b" },
  ];
  const b = a.sort((x, y) => x.k - y.k);
  const out = b.reduce((acc, v) => acc + v.name, "");
  return <Screen title="Home"><Text>{out}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "abc");
}

#[test]
fn array_sort_is_stable_for_equal_keys() {
    // Every item has the same key, so a stable sort must keep the original
    // order; an unstable sort (or one that overwrites equal elements)
    // would not reliably reproduce "abcde" here.
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const a: { k: number; name: string }[] = [
    { k: 1, name: "a" },
    { k: 1, name: "b" },
    { k: 1, name: "c" },
    { k: 1, name: "d" },
    { k: 1, name: "e" },
  ];
  const b = a.sort((x, y) => x.k - y.k);
  const out = b.reduce((acc, v) => acc + v.name, "");
  return <Screen title="Home"><Text>{out}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "abcde");
}

#[test]
fn array_sort_on_empty_and_one_element_arrays() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const empty: number[] = [];
  const one = [42];
  empty.sort((x, y) => x - y);
  one.sort((x, y) => x - y);
  const out = one.reduce((acc, v) => acc + v, "");
  return <Screen title="Home"><Text>{"" + empty.length + "|" + out}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "0|42");
}

#[test]
fn array_sort_ten_thousand_elements() {
    let main = r#"import { app, Screen, Text, signal } from "plinth:ui";
function Home() {
  const n = 10000;
  const a: number[] = [];
  for (let i = 0; i < n; i++) {
    a.push((i * 7919) % n);
  }
  const b = a.sort((x, y) => x - y);
  let ok = true;
  for (let i = 1; i < n; i++) {
    if (b[i - 1] > b[i]) ok = false;
  }
  return <Screen title="Home"><Text>{"" + ok + "|" + b.length}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "true|10000");
}

#[test]
fn array_sort_survives_gc_stress() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const a = [
    { k: 3, name: "c" },
    { k: 1, name: "a" },
    { k: 2, name: "b" },
  ];
  const b = a.sort((x, y) => x.k - y.k);
  const out = b.reduce((acc, v) => acc + v.name, "");
  return <Screen title="Home"><Text>{out}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let fs = MemFs::default().with("app/main.tsx", &main);
    let (front, artifact) = plinth_compiler::compile(&fs).expect("compile");
    let artifact = artifact.unwrap_or_else(|| {
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        panic!("compile errors:\n{}", diags.join("\n"))
    });
    let runner = Runner::new().unwrap();
    let mut guest = runner.load(&artifact.component, Limits::default()).unwrap();
    let mut tree = Tree::new();
    let stress_args = plinth_protocol::init_arg::one(plinth_protocol::init_arg::GC_STRESS, &[]);
    for commit in guest.init(&stress_args).unwrap() {
        let errors = tree.apply(&commit).unwrap();
        assert!(errors.is_empty(), "op errors: {errors:?}");
    }
    assert_eq!(text_of(&tree, ControlKind::Text), "abc");
}

#[test]
fn array_sort_without_a_comparator_on_an_unsupported_element_errors() {
    let main = with_app("const a: { k: number }[] = [{ k: 1 }]; const b = a.sort();");
    assert_eq!(codes(&main), ["PL3006"]);
}

// -- 6. `int | null`, `boolean | null`, enum `| null` (HANDOFF.md item 2) ----
//
// These are boxed through the same box as `number | null` (`BoxI32`/
// `UnboxI32` convert the `i32` to `f64` first, then reuse `box_f64`): no new
// `plinth-rt` function.

#[test]
fn int_or_null_through_params_locals_and_nullish_coalescing() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
import { int } from "plinth:core";
function pick(n: int, useIt: boolean): int | null {
  return useIt ? n : null;
}
function Home() {
  const a: int | null = pick(5, true);
  const b: int | null = pick(5, false);
  const out = "" + (a ?? -1) + "," + (b ?? -1);
  return <Screen title="Home"><Text>{out}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "5,-1");
}

#[test]
fn boolean_or_null_narrowing() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function describe(flag: boolean | null): string {
  if (flag === null) return "none";
  return flag ? "yes" : "no";
}
function Home() {
  const out = describe(true) + "," + describe(false) + "," + describe(null);
  return <Screen title="Home"><Text>{out}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "yes,no,none");
}

#[test]
fn enum_or_null_equality_and_null_check() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
enum Color { Red, Green, Blue }
function Home() {
  const c: Color | null = Color.Green;
  const n: Color | null = null;
  const out = "" + (c === Color.Green) + "," + (n === null) + "," + (c === null);
  return <Screen title="Home"><Text>{out}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "true,true,false");
}

#[test]
fn optional_struct_field_of_int_boxes_through_the_number_box() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
import { int } from "plinth:core";
interface Item { id: int; note?: int; }
function Home() {
  const a: Item = { id: 1, note: 5 };
  const b: Item = { id: 2 };
  const out = "" + (a.note ?? -1) + "," + (b.note ?? -1);
  return <Screen title="Home"><Text>{out}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "5,-1");
}

#[test]
fn map_get_boxes_an_int_value() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
import { int } from "plinth:core";
function Home() {
  const m = new Map<string, int>();
  m.set("a", 3);
  const out = "" + (m.get("a") ?? -1) + "," + (m.get("z") ?? -1);
  return <Screen title="Home"><Text>{out}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "3,-1");
}

#[test]
fn set_get_boxes_a_boolean_map_value() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const m = new Map<string, boolean>();
  m.set("ok", true);
  const out = "" + (m.get("ok") ?? false) + "," + (m.get("missing") ?? false);
  return <Screen title="Home"><Text>{out}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "true,false");
}

#[test]
fn array_of_int_pop_boxes_the_popped_value() {
    // Regression test for a pre-existing bug this item's work uncovered:
    // `Array<T>.pop()` for an `i32`-repr `T` built its `T | null` result by
    // retagging the raw popped scalar as if it were already a boxed
    // reference. It compiled (both are `i32` at the Wasm level) but was
    // wrong at run time for any `T` that needed boxing.
    let main = r#"import { app, Screen, Text } from "plinth:ui";
import { int } from "plinth:core";
function Home() {
  const arr: int[] = [1, 2];
  const p1 = arr.pop() ?? -1;
  const p2 = arr.pop() ?? -1;
  const p3 = arr.pop() ?? -1;
  const out = "" + p1 + "," + p2 + "," + p3;
  return <Screen title="Home"><Text>{out}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "2,1,-1");
}

#[test]
fn int_or_null_switch_on_the_boxed_value() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
import { int } from "plinth:core";
function describe(n: int | null): string {
  switch (n) {
    case 1: return "one";
    case 2: return "two";
    default: return "other";
  }
}
function Home() {
  const out = describe(1) + "," + describe(2) + "," + describe(null) + "," + describe(9);
  return <Screen title="Home"><Text>{out}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "one,two,other,other");
}

#[test]
fn signal_or_null_is_still_rejected() {
    let main = "import { app, Screen, Text, Signal } from \"plinth:ui\";\n\
         function f(s: Signal<number> | null): void {}\n\
         function Home() { return <Screen title=\"Home\" />; }"
        .to_string()
        + APP;
    assert_eq!(codes(&main), ["PL3010"]);
}

// -- Behavior smoke test: confirms the `run` harness works and the lint
// does not fire on a normal counter-style program. -------------------------

#[test]
fn normal_counter_program_compiles_and_runs() {
    let main = r#"import { app, Screen, Text, Button, signal } from "plinth:ui";
const count = signal(0);
function Home() {
  return <Screen title="Home">
    <Text>{count()}</Text>
    <Button label="inc" onPress={() => count.set(count() + 1)} />
  </Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "0");
}

// -- Generic type aliases of object types (HANDOFF.md item 1) --------------

#[test]
fn generic_type_alias_used_in_function_signature() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
type Pair<A, B> = { first: A; second: B };
function swap(p: Pair<number, string>): Pair<string, number> {
  return { first: p.second, second: p.first };
}
function Home() {
  const p: Pair<number, string> = { first: 1, second: "a" };
  const q = swap(p);
  return <Screen title="Home"><Text>{q.first + "-" + q.second}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "a-1");
}

#[test]
fn generic_type_alias_in_variable_annotation() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
type Box<T> = { value: T };
function Home() {
  const b: Box<string> = { value: "hi" };
  return <Screen title="Home"><Text>{b.value}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "hi");
}

#[test]
fn distinct_generic_alias_instantiations_are_distinct_types() {
    // `Pair<number, string>` and `Pair<string, number>` must not be
    // confused with each other (field order/types differ).
    let main = with_app(
        "type Pair<A, B> = { first: A; second: B };\nconst a: Pair<number, string> = { first: 1, second: \"x\" };\nconst b: Pair<string, number> = a;",
    );
    assert_eq!(codes(&main)[0], "PL3001");
}

#[test]
fn generic_type_alias_wrong_type_argument_count() {
    let main = with_app("type Pair<A, B> = { first: A; second: B };\nconst a: Pair<number> = { first: 1, second: 2 };");
    assert_eq!(codes(&main), ["PL3006"]);
}

// -- Generic interfaces, monomorphized (HANDOFF.md item 1, interface case) --

#[test]
fn generic_interface_in_annotation_and_generic_function() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
interface Box<T> { value: T }
function wrap<T>(v: T): Box<T> {
  return { value: v };
}
function Home() {
  const b: Box<number> = { value: 1 };
  const w = wrap("hi");
  return <Screen title="Home"><Text>{b.value + "-" + w.value}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "1-hi");
}

#[test]
fn generic_interface_array_and_nested() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
interface Box<T> { value: T }
function Home() {
  const boxes: Box<number>[] = [{ value: 1 }, { value: 2 }];
  const nested: Box<Box<string>> = { value: { value: "hi" } };
  return <Screen title="Home"><Text>{boxes[0].value + boxes[1].value + "-" + nested.value.value}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "3-hi");
}

#[test]
fn generic_interface_wrong_type_argument_count() {
    let main = with_app("interface Pair<A, B> { first: A; second: B }\nconst a: Pair<number> = { first: 1, second: 2 };");
    assert_eq!(codes(&main), ["PL3006"]);
}

#[test]
fn distinct_generic_interface_instantiations_are_distinct_types() {
    let main = with_app(
        "interface Box<T> { value: T }\nconst a: Box<number> = { value: 1 };\nconst b: Box<string> = a;",
    );
    assert_eq!(codes(&main)[0], "PL3001");
}

// -- Classes (SPEC.md §4.2: a GC ref to a struct; v0 has no `extends`) ------

#[test]
fn counter_class_used_from_jsx_events() {
    let main = r#"import { app, Screen, Text, Button } from "plinth:ui";
class Counter {
  count: number;
  constructor(start: number) { this.count = start; }
  inc(): void { this.count = this.count + 1; }
}
const c = new Counter(0);
function Home() {
  return <Screen title="Home">
    <Text>{c.count}</Text>
    <Button label="inc" onPress={() => c.inc()} />
  </Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "0");
}

#[test]
fn class_with_array_field_and_pushing_method() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
class Box {
  items: string[] = [];
  add(s: string): void { this.items.push(s); }
}
function Home() {
  const b = new Box();
  b.add("a");
  b.add("b");
  return <Screen title="Home"><Text>{b.items.join(",")}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "a,b");
}

#[test]
fn classes_in_an_array() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
class Counter {
  count: number;
  constructor(start: number) { this.count = start; }
}
function Home() {
  const list: Counter[] = [new Counter(1), new Counter(2)];
  return <Screen title="Home"><Text>{list[0].count + list[1].count}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "3");
}

// -- Class inheritance (SPEC.md §4.2 v1: single inheritance) --------------

#[test]
fn shapes_area_through_dynamic_dispatch() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
class Shape {
  name: string;
  constructor(name: string) { this.name = name; }
  area(): number { return 0; }
}
class Circle extends Shape {
  r: number;
  constructor(r: number) { super("circle"); this.r = r; }
  area(): number { return this.r * this.r * 3; }
}
class Square extends Shape {
  side: number;
  constructor(side: number) { super("square"); this.side = side; }
  area(): number { return this.side * this.side; }
}
function total(shapes: Shape[]): number {
  let sum = 0;
  for (const s of shapes) { sum = sum + s.area(); }
  return sum;
}
function Home() {
  const shapes: Shape[] = [new Circle(2), new Square(3)];
  return <Screen title="Home"><Text>{total(shapes)}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    // circle: 2*2*3 = 12, square: 3*3 = 9.
    assert_eq!(text_of(&tree, ControlKind::Text), "21");
}

#[test]
fn base_class_with_no_override_reached_through_subclass_without_area() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
class Shape {
  area(): number { return 5; }
}
class Named extends Shape {
  label: string;
  constructor(label: string) { super(); this.label = label; }
}
function Home() {
  const s: Shape = new Named("x");
  return <Screen title="Home"><Text>{s.area()}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "5");
}

#[test]
fn super_method_call_runs_the_base_implementation() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
class Animal {
  describe(): string { return "an animal"; }
}
class Dog extends Animal {
  describe(): string { return super.describe() + ", a dog"; }
}
function Home() {
  const d = new Dog();
  return <Screen title="Home"><Text>{d.describe()}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "an animal, a dog");
}

#[test]
fn instanceof_narrows_a_base_typed_value() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
class Shape {
  area(): number { return 0; }
}
class Circle extends Shape {
  r: number;
  constructor(r: number) { super(); this.r = r; }
  area(): number { return this.r * this.r * 3; }
  radius(): number { return this.r; }
}
function describe(s: Shape): number {
  if (s instanceof Circle) {
    return s.radius();
  }
  return -1;
}
function Home() {
  const s: Shape = new Circle(4);
  return <Screen title="Home"><Text>{describe(s)}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "4");
}

#[test]
fn instanceof_is_false_for_an_unrelated_subclass() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
class Shape {}
class Circle extends Shape {}
class Square extends Shape {}
function Home() {
  const s: Shape = new Square();
  const isCircle = s instanceof Circle;
  return <Screen title="Home"><Text>{isCircle ? "yes" : "no"}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "no");
}

// -- Narrowing on member expressions (docs/GAPS.md "Found later") ---------

#[test]
fn nullish_coalesce_on_a_member_expression() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
interface Row { subtitle: string | null; }
function sub(r: Row): string { return r.subtitle ?? "x"; }
function Home() {
  const r: Row = { subtitle: null };
  const r2: Row = { subtitle: "hi" };
  return <Screen title="Home"><Text>{sub(r) + "," + sub(r2)}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "x,hi");
}

#[test]
fn nullish_coalesce_on_optional_chaining() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
interface Row { subtitle: string | null; }
function sub(r: Row | null): string { return r?.subtitle ?? "x"; }
function Home() {
  const r: Row = { subtitle: "hi" };
  return <Screen title="Home"><Text>{sub(r) + "," + sub(null)}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "hi,x");
}

#[test]
fn ternary_narrows_a_member_expression() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
interface Row { subtitle: string | null; }
function sub(r: Row): string { return r.subtitle !== null ? r.subtitle : "x"; }
function Home() {
  const r: Row = { subtitle: null };
  const r2: Row = { subtitle: "hi" };
  return <Screen title="Home"><Text>{sub(r) + "," + sub(r2)}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "x,hi");
}

#[test]
fn if_narrows_a_member_expression() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
interface Row { subtitle: string | null; }
function sub(r: Row): string {
  if (r.subtitle !== null) {
    return r.subtitle;
  }
  return "x";
}
function Home() {
  const r: Row = { subtitle: "hi" };
  return <Screen title="Home"><Text>{sub(r)}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "hi");
}

#[test]
fn if_early_return_narrows_a_member_expression_for_the_rest_of_the_block() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
interface Row { subtitle: string | null; }
function sub(r: Row): string {
  if (r.subtitle === null) {
    return "x";
  }
  return r.subtitle;
}
function Home() {
  const r: Row = { subtitle: "hi" };
  return <Screen title="Home"><Text>{sub(r)}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "hi");
}

#[test]
fn two_level_member_path_is_narrowed() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
interface Inner { subtitle: string | null; }
interface Row { inner: Inner; }
function sub(r: Row): string { return r.inner.subtitle !== null ? r.inner.subtitle : "x"; }
function Home() {
  const r: Row = { inner: { subtitle: "hi" } };
  return <Screen title="Home"><Text>{sub(r)}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "hi");
}

#[test]
fn member_narrowing_is_dropped_after_a_call() {
    // Conservative rule (docs/language.md): a call between the test and the
    // use drops member-path narrowing, even though this particular call
    // cannot actually change `r.subtitle`.
    let main = r#"import { app, Screen, Text } from "plinth:ui";
interface Row { subtitle: string | null; }
function noop(): void {}
function sub(r: Row): string {
  if (r.subtitle !== null) {
    noop();
    return r.subtitle;
  }
  return "x";
}
function Home() { return <Screen title="Home"><Text>{""}</Text></Screen>; }
"#
    .to_string()
        + APP;
    assert_eq!(codes(&main), ["PL3001"]);
}

#[test]
fn member_narrowing_is_dropped_after_an_assignment() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
interface Row { subtitle: string | null; }
function sub(r: Row): string {
  if (r.subtitle !== null) {
    r.subtitle = r.subtitle;
    return r.subtitle;
  }
  return "x";
}
function Home() { return <Screen title="Home"><Text>{""}</Text></Screen>; }
"#
    .to_string()
        + APP;
    assert_eq!(codes(&main), ["PL3001"]);
}

#[test]
fn if_early_return_narrows_a_member_expression_inside_a_closure() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
interface Row { subtitle: string | null; }
function Home() {
  const sub = (r: Row): string => {
    const dummy = 1;
    if (r.subtitle === null) {
      return "x";
    }
    return r.subtitle;
  };
  const r: Row = { subtitle: "hi" };
  return <Screen title="Home"><Text>{sub(r)}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "hi");
}

#[test]
fn extends_a_non_class_is_rejected() {
    let main = with_app("interface I { x: number }\nclass C extends I { constructor() { super(); } }");
    assert!(codes(&main).contains(&"PL2022"));
}

#[test]
fn extends_cycle_is_rejected() {
    // `B` is used before it is declared, which also catches a cycle
    // (SPEC.md §4.2 v1): the checker never lets a class extend one that
    // is not fully declared yet.
    let main = with_app("class A extends B { constructor() { super(); } }\nclass B extends A { constructor() { super(); } }");
    assert!(codes(&main).contains(&"PL2022"));
}

#[test]
fn override_with_incompatible_signature_is_rejected() {
    let main = with_app(
        "class Base { m(x: number): number { return x; } }\nclass Sub extends Base { m(x: string): number { return 0; } constructor() { super(); } }",
    );
    assert_eq!(codes(&main), ["PL2024"]);
}

#[test]
fn super_outside_a_subclass_is_rejected() {
    let main = with_app("class C { constructor() { super(); } }");
    assert_eq!(codes(&main), ["PL2023"]);
}

#[test]
fn missing_super_call_is_rejected() {
    let main = with_app("class Base { constructor() {} }\nclass Sub extends Base { x: number = 0; constructor() { this.x = 1; } }");
    assert_eq!(codes(&main), ["PL2023"]);
}

#[test]
fn super_call_not_first_statement_is_rejected() {
    let main =
        with_app("class Base { constructor() {} }\nclass Sub extends Base { x: number = 0; constructor() { this.x = 1; super(); } }");
    assert!(codes(&main).contains(&"PL2023"));
}

#[test]
fn this_outside_a_method_is_rejected() {
    let main = with_app("class C { x: number = 0; constructor() {} }\nfunction f(): number { return this.x; }");
    assert_eq!(codes(&main), ["PL2004"]);
}

#[test]
fn unbound_method_reference_is_rejected() {
    let main = r#"import { app, Screen, Button } from "plinth:ui";
class Counter {
  count: number;
  constructor(start: number) { this.count = start; }
  inc(): void { this.count = this.count + 1; }
}
const c = new Counter(0);
function Home() { return <Screen title="Home"><Button label="inc" onPress={c.inc} /></Screen>; }
"#
    .to_string()
        + APP;
    assert_eq!(codes(&main), ["PL2021"]);
}

// -- JSON (SPEC.md §4.7): `JSON.stringify` for known-shape values. ---------

#[test]
fn json_stringify_scalars() {
    let main = "import { app, Screen, Text } from \"plinth:ui\";\nimport { JSON } from \"plinth:core\";\nfunction Home() { return <Screen title=\"Home\"><Text>{JSON.stringify(1.5) + \"|\" + JSON.stringify(true) + \"|\" + JSON.stringify(false) + \"|\" + JSON.stringify(null)}</Text></Screen>; }"
        .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "1.5|true|false|null");
}

#[test]
fn json_stringify_nan_and_infinity_are_null() {
    let main = "import { app, Screen, Text } from \"plinth:ui\";\nimport { JSON } from \"plinth:core\";\nfunction Home() { return <Screen title=\"Home\"><Text>{JSON.stringify(1 / 0) + \"|\" + JSON.stringify(0 / 0)}</Text></Screen>; }"
        .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "null|null");
}

#[test]
fn json_stringify_string_escapes() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
import { JSON } from "plinth:core";
function Home() { return <Screen title="Home"><Text>{JSON.stringify("a\"b\nc\\d")}</Text></Screen>; }
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "\"a\\\"b\\nc\\\\d\"");
}

#[test]
fn json_stringify_unicode_passes_through() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
import { JSON } from "plinth:core";
function Home() { return <Screen title="Home"><Text>{JSON.stringify("héllo")}</Text></Screen>; }
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "\"héllo\"");
}

#[test]
fn json_stringify_array() {
    let main = "import { app, Screen, Text } from \"plinth:ui\";\nimport { JSON } from \"plinth:core\";\nfunction Home() { return <Screen title=\"Home\"><Text>{JSON.stringify([1, 2, 3])}</Text></Screen>; }"
        .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "[1,2,3]");
}

#[test]
fn json_stringify_empty_array() {
    let main = "import { app, Screen, Text } from \"plinth:ui\";\nimport { JSON } from \"plinth:core\";\nfunction Home() { const a: number[] = []; return <Screen title=\"Home\"><Text>{JSON.stringify(a)}</Text></Screen>; }"
        .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "[]");
}

#[test]
fn json_stringify_nested_array() {
    let main = "import { app, Screen, Text } from \"plinth:ui\";\nimport { JSON } from \"plinth:core\";\nfunction Home() { return <Screen title=\"Home\"><Text>{JSON.stringify([[1, 2], [3]])}</Text></Screen>; }"
        .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "[[1,2],[3]]");
}

#[test]
fn json_stringify_object_field_order() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
import { JSON } from "plinth:core";
interface Point { x: number; y: number; name: string; }
function Home() { const p: Point = { x: 1, y: 2, name: "a" }; return <Screen title="Home"><Text>{JSON.stringify(p)}</Text></Screen>; }
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "{\"x\":1,\"y\":2,\"name\":\"a\"}");
}

#[test]
fn json_stringify_nested_object_and_array() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
import { JSON } from "plinth:core";
interface Item { id: number; tags: string[]; }
function Home() { const it: Item = { id: 1, tags: ["a", "b"] }; return <Screen title="Home"><Text>{JSON.stringify(it)}</Text></Screen>; }
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "{\"id\":1,\"tags\":[\"a\",\"b\"]}");
}

#[test]
fn json_stringify_nullable_fields() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
import { JSON } from "plinth:core";
interface Item { id: number; note: string | null; }
function Home() {
  const a: Item = { id: 1, note: "hi" };
  const b: Item = { id: 2, note: null };
  return <Screen title="Home"><Text>{JSON.stringify(a) + "|" + JSON.stringify(b)}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "{\"id\":1,\"note\":\"hi\"}|{\"id\":2,\"note\":null}");
}

#[test]
fn json_stringify_nullable_number() {
    let main = "import { app, Screen, Text } from \"plinth:ui\";\nimport { JSON } from \"plinth:core\";\nfunction Home() { const n: number | null = null; const m: number | null = 5; return <Screen title=\"Home\"><Text>{JSON.stringify(n) + \"|\" + JSON.stringify(m)}</Text></Screen>; }"
        .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "null|5");
}

#[test]
fn json_stringify_map_as_object() {
    let main = "import { app, Screen, Text } from \"plinth:ui\";\nimport { JSON } from \"plinth:core\";\nfunction Home() { const m = new Map<string, number>(); m.set(\"a\", 1); m.set(\"b\", 2); return <Screen title=\"Home\"><Text>{JSON.stringify(m)}</Text></Screen>; }"
        .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "{\"a\":1,\"b\":2}");
}

#[test]
fn json_stringify_int() {
    let main = "import { app, Screen, Text } from \"plinth:ui\";\nimport { JSON, int } from \"plinth:core\";\nfunction Home() { return <Screen title=\"Home\"><Text>{JSON.stringify(int(7))}</Text></Screen>; }"
        .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "7");
}

#[test]
fn json_stringify_function_is_rejected() {
    let main = "import { app, Screen, Text } from \"plinth:ui\";\nimport { JSON } from \"plinth:core\";\nfunction Home() { const f = () => 1; return <Screen title=\"Home\"><Text>{JSON.stringify(f)}</Text></Screen>; }"
        .to_string()
        + APP;
    assert_eq!(codes(&main), ["PL3001"]);
}

#[test]
fn json_stringify_extra_args_is_rejected() {
    let main = "import { app, Screen, Text } from \"plinth:ui\";\nimport { JSON } from \"plinth:core\";\nfunction Home() { return <Screen title=\"Home\"><Text>{JSON.stringify(1, 2)}</Text></Screen>; }"
        .to_string()
        + APP;
    assert_eq!(codes(&main), ["PL3006"]);
}

// -- JSON.parse<T> (SPEC.md §4.7) ------------------------------------------

#[test]
fn json_parse_scalars_round_trip() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
import { JSON, int } from "plinth:core";
function Home() {
  const n = JSON.parse<number>("42.5");
  const i = JSON.parse<int>("7");
  const b = JSON.parse<boolean>("true");
  const s = JSON.parse<string>("\"hi\"");
  return <Screen title="Home"><Text>{JSON.stringify(n) + "|" + JSON.stringify(i) + "|" + JSON.stringify(b) + "|" + JSON.stringify(s)}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "42.5|7|true|\"hi\"");
}

#[test]
fn json_parse_whitespace_and_escapes() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
import { JSON } from "plinth:core";
function Home() {
  const s = JSON.parse<string>("  \"a\\nb\\u0041\\uD83D\\uDE00\"  ");
  return <Screen title="Home"><Text>{JSON.stringify(s)}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "\"a\\nbA😀\"");
}

#[test]
fn json_parse_null_and_nullable() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
import { JSON } from "plinth:core";
function Home() {
  const a = JSON.parse<number | null>("null");
  const b = JSON.parse<number | null>("5");
  return <Screen title="Home"><Text>{JSON.stringify(a) + "|" + JSON.stringify(b)}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "null|5");
}

#[test]
fn json_parse_array() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
import { JSON } from "plinth:core";
function Home() {
  const a = JSON.parse<number[]>("[1, 2, 3]");
  return <Screen title="Home"><Text>{JSON.stringify(a)}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "[1,2,3]");
}

#[test]
fn json_parse_nested_object_and_array() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
import { JSON } from "plinth:core";
interface Item { id: number; tags: string[]; note: string | null; }
function Home() {
  const it = JSON.parse<Item>("{\"id\":1,\"tags\":[\"a\",\"b\"],\"note\":null}");
  return <Screen title="Home"><Text>{JSON.stringify(it)}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "{\"id\":1,\"tags\":[\"a\",\"b\"],\"note\":null}");
}

#[test]
fn json_parse_object_ignores_extra_fields() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
import { JSON } from "plinth:core";
interface Point { x: number; y: number; }
function Home() {
  const p = JSON.parse<Point>("{\"z\":9,\"x\":1,\"extra\":[1,2,{\"a\":1}],\"y\":2}");
  return <Screen title="Home"><Text>{JSON.stringify(p)}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "{\"x\":1,\"y\":2}");
}

#[test]
fn json_parse_map() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
import { JSON } from "plinth:core";
function Home() {
  const m = JSON.parse<Map<string, number>>("{\"a\":1,\"b\":2}");
  return <Screen title="Home"><Text>{JSON.stringify(m)}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "{\"a\":1,\"b\":2}");
}

#[test]
fn json_parse_bad_syntax_is_null() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
import { JSON } from "plinth:core";
function Home() {
  const n = JSON.parse<number>("not json");
  return <Screen title="Home"><Text>{JSON.stringify(n)}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "null");
}

#[test]
fn json_parse_wrong_type_is_null() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
import { JSON } from "plinth:core";
function Home() {
  const n = JSON.parse<number>("\"hi\"");
  return <Screen title="Home"><Text>{JSON.stringify(n)}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "null");
}

#[test]
fn json_parse_missing_field_is_null() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
import { JSON } from "plinth:core";
interface Point { x: number; y: number; }
function Home() {
  const p = JSON.parse<Point>("{\"x\":1}");
  return <Screen title="Home"><Text>{JSON.stringify(p)}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "null");
}

#[test]
fn json_parse_trailing_garbage_is_null() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
import { JSON } from "plinth:core";
function Home() {
  const n = JSON.parse<number>("1 2");
  return <Screen title="Home"><Text>{JSON.stringify(n)}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "null");
}

/// A recursive interface (a tree) made the compiler recurse until the
/// stack overflowed: the JSON code was inline for each type. Now each
/// struct type has one generated function, which can call itself.
#[test]
fn json_round_trips_a_recursive_type() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
import { JSON } from "plinth:core";
interface Tree { name: string; kids: Tree[]; next: Tree | null; }
function count(t: Tree): number {
  let n = 1;
  for (const k of t.kids) { n += count(k); }
  return t.next === null ? n : n + count(t.next);
}
function Home() {
  const t: Tree = { name: "a", kids: [{ name: "b", kids: [], next: null }, { name: "c", kids: [{ name: "d", kids: [], next: null }], next: null }], next: { name: "e", kids: [], next: null } };
  const text = JSON.stringify(t);
  const back = JSON.parse<Tree>(text);
  const bad = JSON.parse<Tree>("{\"name\":\"x\",\"kids\":[{\"name\":\"y\"}],\"next\":null}");
  const out = back === null ? "null" : JSON.stringify(back) === text ? "same " + count(back) : "different";
  return <Screen title="Home"><Text>{text + "|" + out + "|" + (bad === null ? "bad" : "ok")}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    let tree_json = r#"{"name":"a","kids":[{"name":"b","kids":[],"next":null},{"name":"c","kids":[{"name":"d","kids":[],"next":null}],"next":null}],"next":{"name":"e","kids":[],"next":null}}"#;
    assert_eq!(text_of(&tree, ControlKind::Text), format!("{tree_json}|same 5|bad"));
}

/// A required field of the type itself can never be filled from JSON:
/// the parse gives `null`, and the compiler does not recurse forever.
#[test]
fn json_parse_of_a_type_that_holds_itself_is_null() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
import { JSON } from "plinth:core";
interface Loop { name: string; again: Loop; }
function Home() {
  const v = JSON.parse<Loop>("{\"name\":\"x\",\"again\":{\"name\":\"y\"}}");
  return <Screen title="Home"><Text>{v === null ? "null" : v.name}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "null");
}

#[test]
fn json_parse_int_out_of_range_is_null() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
import { JSON, int } from "plinth:core";
function Home() {
  const i = JSON.parse<int>("1.5");
  return <Screen title="Home"><Text>{JSON.stringify(i)}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "null");
}

#[test]
fn json_stringify_then_parse_round_trips_a_struct() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
import { JSON } from "plinth:core";
interface Item { id: number; tags: string[]; note: string | null; }
function Home() {
  const it: Item = { id: 1, tags: ["a", "b"], note: null };
  const text = JSON.stringify(it);
  const back = JSON.parse<Item>(text);
  return <Screen title="Home"><Text>{JSON.stringify(back)}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "{\"id\":1,\"tags\":[\"a\",\"b\"],\"note\":null}");
}

#[test]
fn json_parse_without_a_type_argument_is_rejected() {
    let main = "import { app, Screen, Text } from \"plinth:ui\";\nimport { JSON } from \"plinth:core\";\nfunction Home() { return <Screen title=\"Home\"><Text>{JSON.parse(\"1\")}</Text></Screen>; }"
        .to_string()
        + APP;
    let codes = codes(&main);
    assert_eq!(codes, ["PL2015"]);
}

// -- Strings (dogfooding gap #4 and neighbors: split, replace(All),
// padStart/padEnd, charAt, lastIndexOf) ------------------------------------

#[test]
fn split_joins_back_with_the_right_pieces() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const parts = "a,bb,ccc".split(",");
  return <Screen title="Home"><Text>{parts.length + ":" + parts.join("|")}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "3:a|bb|ccc");
}

#[test]
fn split_with_empty_separator_splits_per_character_including_non_ascii() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const parts = "aé€".split("");
  return <Screen title="Home"><Text>{parts.length + ":" + parts.join("-")}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    // "aé€" is 3 Unicode scalar values (Plinth TS splits on scalars for an
    // empty separator; `length`/`slice` elsewhere use UTF-16 code units,
    // which agrees here since none of these three characters are astral).
    assert_eq!(text_of(&tree, ControlKind::Text), "3:a-é-€");
}

#[test]
fn replace_only_replaces_the_first_match() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const s = "ab ab ab".replace("ab", "X");
  return <Screen title="Home"><Text>{s}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "X ab ab");
}

#[test]
fn replace_all_replaces_every_match() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const s = "ab ab ab".replaceAll("ab", "X");
  return <Screen title="Home"><Text>{s}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "X X X");
}

#[test]
fn pad_start_and_pad_end_use_the_given_fill() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const s = "7".padStart(3, "0") + "-" + "ab".padEnd(5, "xy");
  return <Screen title="Home"><Text>{s}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "007-abxyx");
}

#[test]
fn pad_default_fill_is_a_space() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  return <Screen title="Home"><Text>{"5".padStart(3) + "|"}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "  5|");
}

#[test]
fn char_at_reads_one_utf16_unit() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const s = "héllo";
  return <Screen title="Home"><Text>{s.charAt(0) + s.charAt(1) + s.charAt(10)}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "hé");
}

// -- Dynamic `Picker.options` (dogfooding gap #1): any `string[]`
// expression, not just an array literal, now works and updates the host
// tree's `options` prop when the underlying signal changes. -------------

#[test]
fn picker_options_from_a_signal_update_at_run_time() {
    let main = r#"import { app, Screen, Picker, Button, signal, computed } from "plinth:ui";
function Home() {
  const wide = signal(false);
  const opts = computed(() => (wide() ? ["a", "b", "c"] : ["a", "b"]));
  const v = signal("a");
  return (
    <Screen title="Home">
      <Picker label="P" value={v} options={opts()} />
      <Button label="Toggle" onPress={() => wide.set(true)} />
    </Screen>
  );
}
"#
    .to_string()
        + APP;
    let fs = MemFs::default().with("app/main.tsx", &main);
    let (front, artifact) = plinth_compiler::compile(&fs).expect("compile");
    let artifact = artifact.unwrap_or_else(|| {
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        panic!("compile errors:\n{}", diags.join("\n"))
    });
    let runner = Runner::new().unwrap();
    let mut guest = runner.load(&artifact.component, Limits::default()).unwrap();
    let mut tree = Tree::new();
    for commit in guest.init(&[]).unwrap() {
        tree.apply(&commit).unwrap();
    }
    let picker = {
        let mut stack: Vec<_> = tree.screens().map(|(_, id)| id).collect();
        loop {
            let id = stack.pop().expect("no Picker found");
            let node = tree.get(id).unwrap();
            if node.kind == Some(ControlKind::Picker) {
                break id;
            }
            stack.extend(node.children.iter());
        }
    };
    assert_eq!(tree.get(picker).unwrap().str_prop(prop::OPTIONS), Some("a\u{1f}b"));

    let button = {
        let mut stack: Vec<_> = tree.screens().map(|(_, id)| id).collect();
        loop {
            let id = stack.pop().expect("no Button found");
            let node = tree.get(id).unwrap();
            if node.kind == Some(ControlKind::Button) {
                break id;
            }
            stack.extend(node.children.iter());
        }
    };
    let handler = tree.get(button).unwrap().handler(event::PRESS).expect("button has a handler");
    let mut w = Writer::new();
    w.event(&Event::Ui { handler, event: event::PRESS, value: Value::Null });
    for commit in guest.on_event(w.as_bytes()).unwrap() {
        tree.apply(&commit).unwrap();
    }
    assert_eq!(tree.get(picker).unwrap().str_prop(prop::OPTIONS), Some("a\u{1f}b\u{1f}c"));
}

/// An array literal of `const` names (7GUIs flight booker:
/// `options={[ONE_WAY, RETURN]}`) is not a literal of strings, so it is
/// joined at run time instead of rejected.
#[test]
fn picker_options_from_an_array_of_const_names() {
    let main = r#"import { app, Screen, Picker, signal } from "plinth:ui";
const A = "one way";
const B = "return";
function Home() {
  const v = signal(A);
  return (
    <Screen title="Home">
      <Picker label="P" value={v} options={[A, B, "x"]} />
    </Screen>
  );
}
"#
    .to_string()
        + APP;
    let fs = MemFs::default().with("app/main.tsx", &main);
    let (front, artifact) = plinth_compiler::compile(&fs).expect("compile");
    let artifact = artifact.unwrap_or_else(|| {
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        panic!("compile errors:\n{}", diags.join("\n"))
    });
    let runner = Runner::new().unwrap();
    let mut guest = runner.load(&artifact.component, Limits::default()).unwrap();
    let mut tree = Tree::new();
    for commit in guest.init(&[]).unwrap() {
        tree.apply(&commit).unwrap();
    }
    let mut stack: Vec<_> = tree.screens().map(|(_, id)| id).collect();
    let picker = loop {
        let id = stack.pop().expect("no Picker found");
        let node = tree.get(id).unwrap();
        if node.kind == Some(ControlKind::Picker) {
            break id;
        }
        stack.extend(node.children.iter());
    };
    assert_eq!(tree.get(picker).unwrap().str_prop(prop::OPTIONS), Some("one way\u{1f}return\u{1f}x"));
}

/// A one-way `value` plus `onChange` (SPEC.md §8.4): the guest does not
/// send the changed value back to the node that sent it, but a row that
/// the change makes again gets the value, also when its new node has the
/// same id as the old one (ids are used again).
#[test]
fn one_way_value_is_not_echoed_but_a_new_row_gets_it() {
    let main = r#"import { app, Screen, List, Toggle, TextField, signal } from "plinth:ui";
interface Item { id: number; on: boolean }
function Home() {
  const items = signal<Item[]>([{ id: 1, on: false }]);
  const text = signal("");
  return (
    <Screen title="Home">
      <TextField label="T" value={text()} onChange={(v) => text.set(v)} />
      <List
        items={items()}
        key={(i) => i.id}
        row={(i) => <Toggle label="On" value={i.on} onChange={(on) => items.set([{ id: i.id, on: on }])} />}
      />
    </Screen>
  );
}
"#
    .to_string()
        + APP;
    let fs = MemFs::default().with("app/main.tsx", &main);
    let (front, artifact) = plinth_compiler::compile(&fs).expect("compile");
    let artifact = artifact.unwrap_or_else(|| {
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        panic!("compile errors:\n{}", diags.join("\n"))
    });
    let runner = Runner::new().unwrap();
    let mut guest = runner.load(&artifact.component, Limits::default()).unwrap();
    let mut tree = Tree::new();
    for commit in guest.init(&[]).unwrap() {
        tree.apply(&commit).unwrap();
    }
    let find = |tree: &Tree, kind: ControlKind| {
        let mut stack: Vec<_> = tree.screens().map(|(_, id)| id).collect();
        loop {
            let id = stack.pop().expect("node not found");
            let node = tree.get(id).unwrap();
            if node.kind == Some(kind) {
                break id;
            }
            stack.extend(node.children.iter());
        }
    };
    let mut change = |tree: &mut Tree, node: u32, value: Value| -> Vec<plinth_protocol::Op> {
        tree.set_local_prop(node, prop::VALUE, value.clone());
        let handler = tree.get(node).unwrap().handler(event::CHANGE).expect("a change handler");
        let mut w = Writer::new();
        w.event(&Event::Ui { handler, event: event::CHANGE, value });
        let mut ops = Vec::new();
        for commit in guest.on_event(w.as_bytes()).unwrap() {
            tree.apply(&commit).unwrap();
            ops.extend(plinth_protocol::decode_ops(&commit).unwrap());
        }
        ops
    };
    let field = find(&tree, ControlKind::TextField);
    let ops = change(&mut tree, field, Value::Str("abc".into()));
    assert!(!ops.iter().any(|op| matches!(op, plinth_protocol::Op::SetProp { prop: p, .. } if *p == prop::VALUE)), "{ops:?}");
    for on in [true, false, true] {
        let toggle = find(&tree, ControlKind::Toggle);
        change(&mut tree, toggle, Value::Bool(on));
        let toggle = find(&tree, ControlKind::Toggle);
        assert_eq!(tree.get(toggle).unwrap().bool_prop(prop::VALUE), on);
    }
}

#[test]
fn last_index_of_finds_the_final_occurrence() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const i = "ab ab ab".lastIndexOf("ab");
  return <Screen title="Home"><Text>{"" + i}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "6");
}

// -- JSX fragments as children (dogfooding gap #3): `<>...</>` creates no
// node of its own; its children splice straight into the parent's child
// list. Allowed only as a child, not as a component's return value. ------

#[test]
fn fragment_as_jsx_child_splices_its_children() {
    let main = r#"import { app, Screen, Section, Text } from "plinth:ui";
function Home() {
  return (
    <Screen title="Home">
      <Section>
        <><Text>{"a"}</Text><Text>{"b"}</Text></>
      </Section>
    </Screen>
  );
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    let mut stack: Vec<_> = tree.screens().map(|(_, id)| id).collect();
    let mut texts = Vec::new();
    while let Some(id) = stack.pop() {
        let node = tree.get(id).unwrap();
        if node.kind == Some(ControlKind::Text) {
            if let Some(t) = &node.text {
                texts.push(t.clone());
            }
        }
        stack.extend(node.children.iter());
    }
    texts.sort();
    assert_eq!(texts, vec!["a".to_string(), "b".to_string()]);
}

#[test]
fn fragment_as_component_return_value_is_rejected() {
    let main = with_app(
        "function Frag() { return <><Text>{\"a\"}</Text></>; }",
    );
    assert_eq!(codes(&main), vec!["PL4004"]);
}

#[test]
fn fragment_inside_a_ternary_branch_is_still_rejected() {
    // Flattening is purely syntactic: a fragment nested inside a dynamic
    // expression (here, one branch of a ternary) is not a direct JSX
    // child, so it is not flattened and is still rejected.
    let main = with_app(
        "const cond = true;\nfunction Frag() { return <Text>{cond ? <><Text>{\"a\"}</Text></> : <Text>{\"b\"}</Text>}</Text>; }",
    );
    assert_eq!(codes(&main), vec!["PL4004"]);
}

// -- Safe `as` casts (dogfooding gap #6): a string to a narrower literal
// union, and a discriminated union to one of its members, are allowed,
// each with its own run-time check that traps if the value does not
// actually match. Casts between unrelated types, and `number as int`,
// are still rejected with PL2006. ----------------------------------------

#[test]
fn string_as_literal_union_narrows_and_runs() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
type Category = "Groceries" | "Rent";
function categorize(s: string): Category {
  return s as Category;
}
function Home() {
  const c = categorize("Rent");
  return <Screen title="Home"><Text>{c}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "Rent");
}

#[test]
fn string_as_literal_union_traps_on_a_bad_value() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
type Category = "Groceries" | "Rent";
function categorize(s: string): Category {
  return s as Category;
}
function Home() {
  const c = categorize("Nope");
  return <Screen title="Home"><Text>{c}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let fs = MemFs::default().with("app/main.tsx", &main);
    let (front, artifact) = plinth_compiler::compile(&fs).expect("compile");
    let artifact = artifact.unwrap_or_else(|| {
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        panic!("compile errors:\n{}", diags.join("\n"))
    });
    let runner = Runner::new().unwrap();
    let mut guest = runner.load(&artifact.component, Limits::default()).unwrap();
    let err = guest.init(&[]).err().expect("expected a trap");
    let msg = format!("{err:?}");
    assert!(msg.to_lowercase().contains("unreachable") || msg.to_lowercase().contains("trap"), "{msg}");
}

#[test]
fn union_as_member_narrows_and_runs() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
interface Circle { kind: "circle"; r: number }
interface Square { kind: "square"; side: number }
type Shape = Circle | Square;
function asCircle(s: Shape): Circle {
  return s as Circle;
}
function Home() {
  const s: Shape = { kind: "circle", r: 2 };
  const c = asCircle(s);
  return <Screen title="Home"><Text>{"" + c.r}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "2");
}

#[test]
fn union_as_wrong_member_traps() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
interface Circle { kind: "circle"; r: number }
interface Square { kind: "square"; side: number }
type Shape = Circle | Square;
function asCircle(s: Shape): Circle {
  return s as Circle;
}
function Home() {
  const s: Shape = { kind: "square", side: 2 };
  const c = asCircle(s);
  return <Screen title="Home"><Text>{"" + c.r}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let fs = MemFs::default().with("app/main.tsx", &main);
    let (front, artifact) = plinth_compiler::compile(&fs).expect("compile");
    let artifact = artifact.unwrap_or_else(|| {
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        panic!("compile errors:\n{}", diags.join("\n"))
    });
    let runner = Runner::new().unwrap();
    let mut guest = runner.load(&artifact.component, Limits::default()).unwrap();
    assert!(guest.init(&[]).is_err(), "expected a trap");
}

#[test]
fn number_as_int_is_rejected() {
    let main = with_app("const n: number = 5; const x = n as int;");
    assert_eq!(codes(&main), vec!["PL2006"]);
}

#[test]
fn as_cast_between_unrelated_types_is_rejected() {
    let main = with_app("const n: number = 5; const x = n as boolean;");
    assert_eq!(codes(&main), vec!["PL2006"]);
}

// -- `.map()` as JSX children (dogfooding gap #2): desugars to the same
// keyed `List` reconciler a literal `<List>` lowers to, keyed by
// position. -----------------------------------------------------------

fn texts_in(tree: &Tree) -> Vec<String> {
    let mut stack: Vec<_> = tree.screens().map(|(_, id)| id).collect();
    let mut texts = Vec::new();
    while let Some(id) = stack.pop() {
        let node = tree.get(id).unwrap();
        if node.kind == Some(ControlKind::Text) {
            if let Some(t) = &node.text {
                texts.push(t.clone());
            }
        }
        stack.extend(node.children.iter());
    }
    texts
}

#[test]
fn map_as_jsx_child_renders_all_items() {
    let main = r#"import { app, Screen, Section, Text, signal } from "plinth:ui";
function Home() {
  const items = signal(["a", "b", "c"]);
  return (
    <Screen title="Home">
      <Section>
        {items().map((x) => <Text>{x}</Text>)}
      </Section>
    </Screen>
  );
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    let mut texts = texts_in(&tree);
    texts.sort();
    assert_eq!(texts, vec!["a".to_string(), "b".to_string(), "c".to_string()]);
}

#[test]
fn filter_then_map_as_jsx_child_is_allowed() {
    let main = r#"import { app, Screen, Section, Text, signal } from "plinth:ui";
function Home() {
  const items = signal([1, 2, 3, 4]);
  return (
    <Screen title="Home">
      <Section>
        {items().filter((x) => x % 2 === 0).map((x) => <Text>{"" + x}</Text>)}
      </Section>
    </Screen>
  );
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    let mut texts = texts_in(&tree);
    texts.sort();
    assert_eq!(texts, vec!["2".to_string(), "4".to_string()]);
}

#[test]
fn map_as_jsx_child_updates_on_add_remove_and_reorder() {
    let main = r#"import { app, Screen, Section, Text, Button, signal } from "plinth:ui";
function Home() {
  const items = signal(["a", "b", "c"]);
  return (
    <Screen title="Home">
      <Section>
        {items().map((x) => <Text>{x}</Text>)}
      </Section>
      <Button label="Push" onPress={() => items.set([...items(), "d"])} />
      <Button label="Shift" onPress={() => { const xs = items().slice(); xs.reverse(); items.set(xs); }} />
      <Button label="Pop" onPress={() => items.set(items().slice(0, items().length - 1))} />
    </Screen>
  );
}
"#
    .to_string()
        + APP;
    let fs = MemFs::default().with("app/main.tsx", &main);
    let (front, artifact) = plinth_compiler::compile(&fs).expect("compile");
    let artifact = artifact.unwrap_or_else(|| {
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        panic!("compile errors:\n{}", diags.join("\n"))
    });
    let runner = Runner::new().unwrap();
    let mut guest = runner.load(&artifact.component, Limits::default()).unwrap();
    let mut tree = Tree::new();
    for commit in guest.init(&[]).unwrap() {
        let errors = tree.apply(&commit).unwrap();
        assert!(errors.is_empty(), "op errors: {errors:?}");
    }

    fn press(guest: &mut plinth_runner_wasmtime::Guest, tree: &mut Tree, label: &str) {
        let button = {
            let mut stack: Vec<_> = tree.screens().map(|(_, id)| id).collect();
            loop {
                let id = stack.pop().expect("no Button found");
                let node = tree.get(id).unwrap();
                if node.kind == Some(ControlKind::Button) && node.str_prop(prop::LABEL) == Some(label) {
                    break id;
                }
                stack.extend(node.children.iter());
            }
        };
        let handler = tree.get(button).unwrap().handler(event::PRESS).expect("button has a handler");
        let mut w = Writer::new();
        w.event(&Event::Ui { handler, event: event::PRESS, value: Value::Null });
        for commit in guest.on_event(w.as_bytes()).unwrap() {
            let errors = tree.apply(&commit).unwrap();
            assert!(errors.is_empty(), "op errors: {errors:?}");
        }
    }

    let mut initial = texts_in(&tree);
    initial.sort();
    assert_eq!(initial, vec!["a", "b", "c"]);
    press(&mut guest, &mut tree, "Push");
    let mut after_push = texts_in(&tree);
    after_push.sort();
    assert_eq!(after_push, vec!["a", "b", "c", "d"]);
    press(&mut guest, &mut tree, "Shift");
    let mut after_reverse = texts_in(&tree);
    after_reverse.sort();
    assert_eq!(after_reverse, vec!["a", "b", "c", "d"]);
    press(&mut guest, &mut tree, "Pop");
    let mut after_pop = texts_in(&tree);
    after_pop.sort();
    assert_eq!(after_pop, vec!["b", "c", "d"]);
}

#[test]
fn map_as_jsx_child_survives_gc_stress() {
    let main = r#"import { app, Screen, Section, Text, Button, signal } from "plinth:ui";
function Home() {
  const items = signal(["a", "b", "c"]);
  return (
    <Screen title="Home">
      <Section>
        {items().map((x) => <Text>{x}</Text>)}
      </Section>
      <Button label="Push" onPress={() => items.set([...items(), "d"])} />
    </Screen>
  );
}
"#
    .to_string()
        + APP;
    let fs = MemFs::default().with("app/main.tsx", &main);
    let (front, artifact) = plinth_compiler::compile(&fs).expect("compile");
    let artifact = artifact.unwrap_or_else(|| {
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        panic!("compile errors:\n{}", diags.join("\n"))
    });
    let runner = Runner::new().unwrap();
    let mut guest = runner.load(&artifact.component, Limits::default()).unwrap();
    let mut tree = Tree::new();
    let stress_args = plinth_protocol::init_arg::one(plinth_protocol::init_arg::GC_STRESS, &[]);
    for commit in guest.init(&stress_args).unwrap() {
        let errors = tree.apply(&commit).unwrap();
        assert!(errors.is_empty(), "op errors: {errors:?}");
    }
    let button = {
        let mut stack: Vec<_> = tree.screens().map(|(_, id)| id).collect();
        loop {
            let id = stack.pop().expect("no Button found");
            let node = tree.get(id).unwrap();
            if node.kind == Some(ControlKind::Button) {
                break id;
            }
            stack.extend(node.children.iter());
        }
    };
    let handler = tree.get(button).unwrap().handler(event::PRESS).expect("button has a handler");
    let mut w = Writer::new();
    w.event(&Event::Ui { handler, event: event::PRESS, value: Value::Null });
    for commit in guest.on_event(w.as_bytes()).unwrap() {
        let errors = tree.apply(&commit).unwrap();
        assert!(errors.is_empty(), "op errors: {errors:?}");
    }
}

#[test]
fn map_with_a_named_function_reference_is_still_rejected() {
    // Only an inline arrow/function is desugared; a named reference falls
    // through to the ordinary "array is not a child" diagnostic.
    let main = with_app("function row(x: string) { return <Text>{x}</Text>; }\nconst items: string[] = [\"a\", \"b\"];")
        .replace(
            "import { app, Screen, Text, signal, computed, effect } from \"plinth:ui\";",
            "import { app, Screen, Section, Text, signal, computed, effect } from \"plinth:ui\";",
        )
        .replace("<Screen title=\"Home\" />", "<Screen title=\"Home\"><Section>{items.map(row)}</Section></Screen>");
    assert_eq!(codes(&main), vec!["PL4004"]);
}

/// docs/GAPS.md G8: an effect that gives the same prop value or text again
/// sends no op; a changed value is sent; `value` is always sent (the host
/// can change it), so a field can be reset to the same text.
#[test]
fn unchanged_props_and_text_are_not_sent_again() {
    let main = r#"import { app, Screen, Button, Text, TextField, signal } from "plinth:ui";
function Home() {
  const n = signal(0);
  const typed = signal("");
  return (
    <Screen title="Home">
      <Text tone={n() > 1 ? "danger" : "default"}>{n() > 1 ? "big" : "small"}</Text>
      <TextField label="T" value={typed()} onChange={(v) => typed.set(v)} onSubmit={() => typed.set("")} />
      <Button label="Add" onPress={() => n.set(n() + 1)} />
    </Screen>
  );
}
"#
    .to_string()
        + APP;
    let fs = MemFs::default().with("app/main.tsx", &main);
    let (front, artifact) = plinth_compiler::compile(&fs).expect("compile");
    let artifact = artifact.unwrap_or_else(|| {
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        panic!("compile errors:\n{}", diags.join("\n"))
    });
    let runner = Runner::new().unwrap();
    let mut guest = runner.load(&artifact.component, Limits::default()).unwrap();
    let mut tree = Tree::new();
    for commit in guest.init(&[]).unwrap() {
        tree.apply(&commit).unwrap();
    }
    let find = |tree: &Tree, kind: ControlKind| {
        let mut stack: Vec<_> = tree.screens().map(|(_, id)| id).collect();
        loop {
            let id = stack.pop().expect("node not found");
            let node = tree.get(id).unwrap();
            if node.kind == Some(kind) {
                break id;
            }
            stack.extend(node.children.iter());
        }
    };
    let mut fire = |tree: &mut Tree, node: u32, ev: u16, value: Value| -> Vec<plinth_protocol::Op> {
        let handler = tree.get(node).unwrap().handler(ev).expect("a handler");
        let mut w = Writer::new();
        w.event(&Event::Ui { handler, event: ev, value });
        let mut ops = Vec::new();
        for commit in guest.on_event(w.as_bytes()).unwrap() {
            tree.apply(&commit).unwrap();
            ops.extend(plinth_protocol::decode_ops(&commit).unwrap());
        }
        ops
    };
    let button = find(&tree, ControlKind::Button);
    // 0 -> 1: the text and the tone do not change: no op for them.
    let ops = fire(&mut tree, button, event::PRESS, Value::Null);
    assert!(!ops.iter().any(|op| matches!(op, plinth_protocol::Op::Text { .. } | plinth_protocol::Op::SetProp { .. })), "{ops:?}");
    // 1 -> 2: both change.
    let ops = fire(&mut tree, button, event::PRESS, Value::Null);
    assert!(ops.iter().any(|op| matches!(op, plinth_protocol::Op::Text { .. })), "{ops:?}");
    assert!(ops.iter().any(|op| matches!(op, plinth_protocol::Op::SetProp { prop: p, .. } if *p == prop::TONE)), "{ops:?}");
    // `value` is always sent: type "x", then Enter resets the field to "".
    let field = find(&tree, ControlKind::TextField);
    tree.set_local_prop(field, prop::VALUE, Value::Str("x".into()));
    fire(&mut tree, field, event::CHANGE, Value::Str("x".into()));
    let ops = fire(&mut tree, field, event::SUBMIT, Value::Str("x".into()));
    assert!(ops.iter().any(|op| matches!(op, plinth_protocol::Op::SetProp { prop: p, value: Value::Str(s), .. } if *p == prop::VALUE && s.is_empty())), "{ops:?}");
}

/// `const [a, ...rest] = xs` (array and tuple rest patterns).
#[test]
fn array_rest_patterns_bind_the_remaining_elements() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const nums = [1, 2, 3, 4];
  const [first, , ...others] = nums;
  const [w, ...ws] = ["a", "b", "c"];
  const [only, ...none] = [7];
  const t: [string, ...number[]] = ["n", 5, 6, 7];
  const [label, x, ...more] = t;
  const out = `${first}|${others.map((n) => `${n}`).join(",")}|${w}${ws.join("")}|${only}:${none.length}|${label}${x}:${more.map((n) => `${n}`).join(",")}`;
  return <Screen title="Home"><Text>{out}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "1|3,4|abc|7:0|n5:6,7");
}

/// `join` on number and boolean arrays (a literal separator), as JavaScript.
#[test]
fn join_converts_numbers_and_booleans() {
    let main = r#"import { app, Screen, Text } from "plinth:ui";
function Home() {
  const xs = [1, 2.5, -3];
  const flags = [true, false];
  const empty: number[] = [];
  const out = `${xs.join(", ")}|${flags.join()}|${[0.1 + 0.2].join("")}|${empty.join("-")}`;
  return <Screen title="Home"><Text>{out}</Text></Screen>;
}
"#
    .to_string()
        + APP;
    let tree = run(&main);
    assert_eq!(text_of(&tree, ControlKind::Text), "1, 2.5, -3|true,false|0.30000000000000004|");
}

/// A narrowed union is a value of the full union again (found by 7GUIs
/// Cells): after `if (e.kind === "range") ... else`, `e` passes to a
/// function that takes the whole union; also through recursive interfaces.
#[test]
fn a_narrowed_union_converts_back_to_the_full_union() {
    let tree = run(r#"
import { app, Screen, Text } from "plinth:ui";

interface Num { kind: "num"; value: number }
interface Range { kind: "range"; from: number; to: number }
interface Add { kind: "add"; args: Expr[] }
type Expr = Num | Range | Add;

function value(e: Expr): number {
  if (e.kind === "num") {
    return e.value;
  }
  if (e.kind === "range") {
    return e.to - e.from;
  }
  let t = 0;
  for (const a of e.args) {
    t = t + (a.kind === "range" ? 100 : value(a));
  }
  return t;
}

function Home() {
  const e: Expr = { kind: "add", args: [{ kind: "num", value: 2 }, { kind: "range", from: 1, to: 4 }, { kind: "add", args: [{ kind: "num", value: 5 }] }] };
  return (
    <Screen title="Home">
      <Text>{`${value(e)}`}</Text>
    </Screen>
  );
}

export default app({ screens: { home: { title: "Home", component: Home } } });
"#);
    assert_eq!(text_of(&tree, ControlKind::Text), "107");
}
