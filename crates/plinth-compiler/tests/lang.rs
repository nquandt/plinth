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
