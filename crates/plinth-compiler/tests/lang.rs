//! Compiler language-gap tests (HANDOFF.md §9 item 3). Separate from
//! `e2e.rs` so this agent does not collide with others editing that file.
//!
//! Diagnostic tests use an in-memory file system; behavior tests compile a
//! small Plinth TS program and run it in wasmtime, like `e2e.rs` does for
//! the example apps.

use plinth_compiler::driver::{Frontend, MemFs, frontend};
use plinth_protocol::ControlKind;
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
    let main = with_app("const m = new Map<string, number>(); for (const x of m) {}");
    assert_eq!(codes(&main), ["PL3001"]);
}

#[test]
fn set_entries_is_not_supported() {
    let main = with_app("const s = new Set<number>(); for (const x of s.entries()) {}");
    assert_eq!(codes(&main), ["PL3004"]);
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
