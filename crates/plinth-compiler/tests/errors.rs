//! `try`/`catch`/`finally`/`throw` (SPEC.md §5.6, core 1.9). A thrown
//! value is an `Error` (or a subclass); runtime traps are not catchable; an
//! uncaught exception in an event handler is reported through
//! `error.report` and the app keeps running.

use plinth_compiler::driver::{Frontend, MemFs, frontend};
use plinth_protocol::{ControlKind, Event, NodeId, Value, Writer, event};
use plinth_runner_wasmtime::{Guest, Limits, Runner};
use plinth_ui::tree::{Node, Tree};

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

fn compile(main: &str) -> plinth_compiler::Artifact {
    let fs = MemFs::default().with("app/main.tsx", main);
    let (front, artifact) = plinth_compiler::compile(&fs).expect("compile");
    artifact.unwrap_or_else(|| panic!("compile errors:\n{}", render(&front)))
}

/// Compiles and starts a program; returns the guest and its tree.
fn start_with(main: &str, stress: bool) -> (Guest, Tree) {
    let artifact = compile(main);
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
    (guest, tree)
}

fn start(main: &str) -> (Guest, Tree) {
    start_with(main, false)
}

fn find(tree: &Tree, kind: ControlKind, pred: impl Fn(&Node) -> bool) -> NodeId {
    let mut stack: Vec<NodeId> = tree.screens().map(|(_, id)| id).collect();
    while let Some(id) = stack.pop() {
        let node = tree.get(id).unwrap();
        if node.kind == Some(kind) && pred(node) {
            return id;
        }
        stack.extend(node.children.iter());
    }
    panic!("no matching {kind:?} found");
}

fn text(tree: &Tree) -> String {
    let id = find(tree, ControlKind::Text, |_| true);
    tree.get(id).unwrap().text.clone().unwrap()
}

fn button(tree: &Tree, label: &str) -> NodeId {
    find(tree, ControlKind::Button, |n| n.str_prop(plinth_protocol::prop::LABEL) == Some(label))
}

/// Presses a button; returns the result of the `on-event` call.
fn press(guest: &mut Guest, tree: &mut Tree, label: &str) -> anyhow::Result<()> {
    let id = button(tree, label);
    let handler = tree.get(id).and_then(|n| n.handler(event::PRESS)).expect("button has a handler");
    let mut w = Writer::new();
    w.event(&Event::Ui { handler, event: event::PRESS, value: Value::Null });
    let commits = guest.on_event(w.as_bytes())?;
    for log in guest.take_logs() {
        eprintln!("guest: {log}");
    }
    for commit in commits {
        let errors = tree.apply(&commit).unwrap();
        assert!(errors.is_empty(), "op errors: {errors:?}");
    }
    Ok(())
}

/// A program whose `Home` shows `out`, computed from `body`; `top` holds
/// top-level declarations.
fn show(top: &str, body: &str, out: &str) -> String {
    format!(
        "import {{ app, Screen, Text }} from \"plinth:ui\";\n{top}\nfunction Home() {{\n{body}\n  return <Screen title=\"Home\"><Text>{{{out}}}</Text></Screen>;\n}}\n{APP}"
    )
}

fn shown(top: &str, body: &str, out: &str) -> String {
    let (_, tree) = start(&show(top, body, out));
    text(&tree)
}

// -- try / catch / throw ------------------------------------------------------

#[test]
fn a_thrown_error_reaches_the_catch() {
    let out = shown("", "let m = \"none\";\n  try { throw new Error(\"boom\"); m = \"after\"; } catch (e) { m = e.name + \": \" + e.message; }", "m");
    assert_eq!(out, "Error: boom");
}

#[test]
fn a_thrown_string_becomes_an_error() {
    let out = shown("", "let m = \"none\";\n  try { throw \"plain\"; } catch (e) { m = e.name + \"/\" + e.message; }", "m");
    assert_eq!(out, "Error/plain");
}

#[test]
fn an_exception_leaves_nested_calls() {
    let top = "function inner(n: number): number { if (n > 2) { throw new Error(\"too big: \" + n); } return n * 10; }\n\
               function outer(n: number): number { const a = inner(n); return a + 1; }";
    let body = "let log = \"\";\n  for (const n of [1, 3, 2]) { try { log = log + outer(n) + \",\"; } catch (e) { log = log + \"[\" + e.message + \"],\"; } }";
    assert_eq!(shown(top, body, "log"), "11,[too big: 3],21,");
}

#[test]
fn an_exception_leaves_a_closure_and_an_array_callback() {
    let top = "function check(x: number): number { if (x < 0) { throw new Error(\"negative\"); } return x; }";
    let body = "let m = \"\";\n  try { const r = [1, -2, 3].map((x) => check(x)); m = \"ok \" + r.length; } catch (e) { m = \"caught \" + e.message; }\n  \
                const f = (x: number): number => check(x) + 1;\n  try { m = m + \"|\" + f(4) + \"|\" + f(-1); } catch (e) { m = m + \"|caught2\"; }";
    assert_eq!(shown(top, body, "m"), "caught negative|caught2");
}

#[test]
fn an_exception_leaves_a_method() {
    let top = "class Account { balance: number = 0;\n  withdraw(n: number): void { if (n > this.balance) { throw new Error(\"insufficient\"); } this.balance = this.balance - n; } }";
    let body = "const a = new Account(); a.balance = 5; let m = \"\";\n  try { a.withdraw(2); a.withdraw(9); m = \"no\"; } catch (e) { m = e.message + \" \" + a.balance; }";
    assert_eq!(shown(top, body, "m"), "insufficient 3");
}

#[test]
fn a_subclass_narrows_with_instanceof() {
    let top = "class NotFound extends Error {\n  id: string;\n  constructor(id: string) { super(\"missing \" + id); this.name = \"NotFound\"; this.id = id; }\n}\n\
               function load(id: string): string { if (id === \"x\") { throw new NotFound(id); } if (id === \"y\") { throw new Error(\"bad\"); } return id; }";
    let body = "let m = \"\";\n  for (const id of [\"a\", \"x\", \"y\"]) {\n    try { m = m + load(id) + \";\"; }\n    catch (e) { if (e instanceof NotFound) { m = m + \"nf:\" + e.id + \":\" + e.name + \";\"; } else { m = m + \"other:\" + e.message + \";\"; } }\n  }";
    assert_eq!(shown(top, body, "m"), "a;nf:x:NotFound;other:bad;");
}

#[test]
fn a_catch_can_throw_again_to_an_outer_catch() {
    let body = "let m = \"\";\n  try { try { throw new Error(\"one\"); } catch (e) { m = m + \"inner \"; throw new Error(e.message + \" two\"); } } catch (e) { m = m + \"outer \" + e.message; }";
    assert_eq!(shown("", body, "m"), "inner outer one two");
}

#[test]
fn catch_without_a_binding() {
    let body = "let m = \"a\";\n  try { throw new Error(\"x\"); } catch { m = \"b\"; }";
    assert_eq!(shown("", body, "m"), "b");
}

// -- finally -----------------------------------------------------------------

#[test]
fn finally_runs_on_every_path() {
    let top = "let log = \"\";\n\
               function f(mode: number): number {\n  try {\n    if (mode === 1) { return 1; }\n    if (mode === 2) { throw new Error(\"e\"); }\n    log = log + \"body,\";\n  } catch (e) {\n    log = log + \"catch,\";\n    return 2;\n  } finally {\n    log = log + \"finally\" + mode + \",\";\n  }\n  return 0;\n}";
    let body = "const r = \"\" + f(0) + f(1) + f(2);";
    assert_eq!(shown(top, body, "r + \"|\" + log"), "012|body,finally0,finally1,catch,finally2,");
}

#[test]
fn finally_without_catch_throws_again() {
    let top = "let log = \"\";\nfunction f(): void { try { throw new Error(\"x\"); } finally { log = log + \"cleanup,\"; } }";
    let body = "try { f(); log = log + \"no,\"; } catch (e) { log = log + \"caught \" + e.message; }";
    assert_eq!(shown(top, body, "log"), "cleanup,caught x");
}

#[test]
fn finally_runs_on_break_and_continue() {
    let body = "let log = \"\";\n  for (const i of [1, 2, 3, 4]) {\n    try { if (i === 2) { continue; } if (i === 4) { break; } log = log + i; } finally { log = log + \"f\" + i + \",\"; }\n  }\n  \
                let j = 0;\n  while (j < 3) { j = j + 1; try { if (j === 2) { continue; } log = log + \"w\" + j; } finally { log = log + \".\"; } }";
    assert_eq!(shown("", body, "log"), "1f1,f2,3f3,f4,w1..w3.");
}

#[test]
fn an_exception_in_finally_replaces_the_pending_one() {
    let body = "let m = \"\";\n  try { try { throw new Error(\"first\"); } finally { throw new Error(\"second\"); } } catch (e) { m = e.message; }";
    assert_eq!(shown("", body, "m"), "second");
}

#[test]
fn a_function_that_returns_in_try_and_catch_returns_on_every_path() {
    let top = "function parse(s: string): number { try { if (s === \"\") { throw new Error(\"empty\"); } return s.length; } catch (e) { return -1; } }";
    assert_eq!(shown(top, "const r = \"\" + parse(\"abc\") + parse(\"\");", "r"), "3-1");
}

// -- Uncaught exceptions -------------------------------------------------------

const HANDLER_SRC: &str = r#"import { app, Screen, Text, Button, signal } from "plinth:ui";
const count = signal(0);
function fail(): void { throw new Error("handler failed"); }
function Home() {
  return <Screen title="Home">
    <Text>{`count ${count()}`}</Text>
    <Button label="fail" onPress={() => { count.set(count() + 100); fail(); count.set(-1); }} />
    <Button label="inc" onPress={() => count.set(count() + 1)} />
  </Screen>;
}
export default app({ screens: { home: { title: "Home", component: Home } } });
"#;

#[test]
fn an_uncaught_exception_in_a_handler_is_reported_and_the_app_keeps_running() {
    let (mut guest, mut tree) = start(HANDLER_SRC);
    press(&mut guest, &mut tree, "fail").expect("an uncaught throw must not trap");
    assert_eq!(guest.take_errors(), vec!["Uncaught Error: handler failed".to_string()]);
    // The write before the throw stays; the one after it never ran.
    assert_eq!(text(&tree), "count 100");
    press(&mut guest, &mut tree, "inc").expect("the app still runs");
    assert_eq!(text(&tree), "count 101");
    assert!(guest.take_errors().is_empty());
}

#[test]
fn an_uncaught_exception_survives_gc_stress() {
    let (mut guest, mut tree) = start_with(HANDLER_SRC, true);
    for _ in 0..3 {
        press(&mut guest, &mut tree, "fail").unwrap();
    }
    assert_eq!(guest.take_errors().len(), 3);
    press(&mut guest, &mut tree, "inc").unwrap();
    assert_eq!(text(&tree), "count 301");
}

/// The start-up code has no complete UI after an uncaught exception, so
/// the app reports it and then stops (the host shows "This app stopped").
#[test]
fn an_uncaught_exception_at_module_init_is_reported_then_stops() {
    let main = show("function boom(): number { throw new Error(\"init\"); }\nconst x = boom();", "", "\"never\"");
    let artifact = compile(&main);
    let runner = Runner::new().unwrap();
    let mut guest = runner.load(&artifact.component, Limits::default()).unwrap();
    assert!(guest.init(&[]).is_err());
    assert_eq!(guest.take_errors(), vec!["Uncaught Error: init".to_string()]);
}

#[test]
fn a_subclass_name_appears_in_the_report() {
    let main = r#"import { app, Screen, Button } from "plinth:ui";
class Oops extends Error { constructor() { super("custom"); this.name = "Oops"; } }
function Home() {
  return <Screen title="Home"><Button label="go" onPress={() => { throw new Oops(); }} /></Screen>;
}
export default app({ screens: { home: { title: "Home", component: Home } } });
"#;
    let (mut guest, mut tree) = start(main);
    press(&mut guest, &mut tree, "go").unwrap();
    assert_eq!(guest.take_errors(), vec!["Uncaught Oops: custom".to_string()]);
}

// -- Traps are not exceptions ---------------------------------------------------

#[test]
fn a_runtime_trap_is_not_catchable() {
    let main = r#"import { app, Screen, Text, Button, signal } from "plinth:ui";
const m = signal("start");
function Home() {
  return <Screen title="Home">
    <Text>{m()}</Text>
    <Button label="go" onPress={() => {
      const a: number[] = [];
      try { m.set("v" + a[5]); } catch (e) { m.set("caught"); }
    }} />
  </Screen>;
}
export default app({ screens: { home: { title: "Home", component: Home } } });
"#;
    let (mut guest, mut tree) = start(main);
    assert!(press(&mut guest, &mut tree, "go").is_err(), "an out-of-bounds read traps, even inside `try`");
}

#[test]
fn a_failed_as_cast_is_not_catchable() {
    let main = r#"import { app, Screen, Text, Button, signal } from "plinth:ui";
type Color = "red" | "blue";
const m = signal("green");
function Home() {
  return <Screen title="Home">
    <Text>{m()}</Text>
    <Button label="go" onPress={() => {
      try { const c: Color = m() as Color; m.set(c); } catch (e) { m.set("caught"); }
    }} />
  </Screen>;
}
export default app({ screens: { home: { title: "Home", component: Home } } });
"#;
    let (mut guest, mut tree) = start(main);
    assert!(press(&mut guest, &mut tree, "go").is_err());
}

// -- Cost --------------------------------------------------------------------------

#[test]
fn an_app_without_throw_does_not_need_core_1_9() {
    let a = compile(&show("", "const x = 1;", "\"\" + x"));
    assert!(!a.runtime.ends_with(".9"), "runtime {}", a.runtime);
    let b = compile(&show("", "let m = \"\"; try { m = \"a\"; } catch (e) { m = e.message; }", "m"));
    assert!(!b.runtime.ends_with(".9"), "a try with no throw reaches no new function: {}", b.runtime);
    let c = compile(HANDLER_SRC);
    assert!(c.runtime.ends_with("1.9"), "an uncaught throw needs `uncaught`: {}", c.runtime);
}

// -- Diagnostics -----------------------------------------------------------------

#[test]
fn throwing_a_number_is_rejected() {
    assert_eq!(codes(&show("", "if (1 > 2) { throw 42; }", "\"\"")), vec!["PL3001"]);
}

#[test]
fn a_typed_catch_variable_is_rejected() {
    assert_eq!(codes(&show("", "try { } catch (e: string) { }", "\"\"")), vec!["PL2010"]);
}

#[test]
fn a_catch_variable_typed_unknown_is_allowed() {
    let out = shown("", "let m = \"\";\n  try { throw new Error(\"u\"); } catch (e: unknown) { if (e instanceof Error) { m = e.message; } }", "m");
    assert_eq!(out, "u");
}

#[test]
fn a_destructured_catch_variable_is_rejected() {
    assert_eq!(codes(&show("", "try { } catch ({ message }) { }", "\"\"")), vec!["PL2010"]);
}
