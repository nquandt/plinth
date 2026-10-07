//! `try`/`catch`/`finally`/`throw` (SPEC.md §5.6, core 1.10). A thrown
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
    let e = press(&mut guest, &mut tree, "go").expect_err("an out-of-bounds read traps, even inside `try`");
    assert_eq!(e.to_string(), "array index out of bounds", "the host shows the core's reason: {e:#}");
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

// -- Limits: a bad app stops; the host and other apps keep running -----------------

/// A program with a "go" button that runs `body`, and a "ping" button
/// that changes the text.
fn limit_app(body: &str) -> String {
    format!(
        r#"import {{ app, Screen, Text, Button, signal }} from "plinth:ui";
const m = signal("start");
function Home() {{
  return <Screen title="Home">
    <Text>{{m()}}</Text>
    <Button label="go" onPress={{() => {{ {body} }}}} />
    <Button label="ping" onPress={{() => m.set("pong")}} />
  </Screen>;
}}
export default app({{ screens: {{ home: {{ title: "Home", component: Home }} }} }});
"#
    )
}

fn start_limited(runner: &Runner, main: &str, limits: Limits) -> (Guest, Tree) {
    let artifact = compile(main);
    let mut guest = runner.load(&artifact.component, limits).unwrap();
    let mut tree = Tree::new();
    for commit in guest.init(&[]).unwrap() {
        tree.apply(&commit).unwrap();
    }
    (guest, tree)
}

const SHORT: Limits = Limits { memory_bytes: 16 << 20, call_timeout: std::time::Duration::from_millis(300) };

/// Checks that the guest stopped with an error that contains `want`, that
/// a later call fails at once, and that a second app on the same runner
/// still runs.
fn assert_stops(runner: &Runner, mut guest: Guest, mut tree: Tree, want: &str) {
    let t = std::time::Instant::now();
    let e = press(&mut guest, &mut tree, "go").expect_err("the call must fail");
    // The top message is the plain reason that the host shows.
    let msg = e.to_string();
    assert!(msg.contains(want), "error {msg:?} does not contain {want:?}");
    assert!(!msg.contains('\n'), "the reason is one line: {msg:?}");
    assert!(t.elapsed() < std::time::Duration::from_secs(10), "took {:?}", t.elapsed());
    let e = press(&mut guest, &mut tree, "ping").expect_err("a stopped app is not called again");
    assert!(format!("{e:#}").contains("stopped earlier"), "{e:#}");
    let (mut other, mut other_tree) = start_limited(runner, &limit_app(""), SHORT);
    press(&mut other, &mut other_tree, "ping").unwrap();
    assert_eq!(text(&other_tree), "pong");
}

#[test]
fn an_endless_loop_hits_the_time_limit() {
    let runner = Runner::new().unwrap();
    let (guest, tree) = start_limited(&runner, &limit_app("let i = 0; while (i >= 0) { i = i + 1; }"), SHORT);
    assert_stops(&runner, guest, tree, "not responding");
}

#[test]
fn an_endless_loop_at_start_hits_the_time_limit() {
    let runner = Runner::new().unwrap();
    let main = show("function spin(): number { let i = 0; while (i >= 0) { i = i + 1; } return i; }\nconst x = spin();", "", "\"\" + x");
    let mut guest = runner.load(&compile(&main).component, SHORT).unwrap();
    let e = guest.init(&[]).expect_err("init must fail");
    assert!(format!("{e:#}").contains("not responding"), "{e:#}");
}

#[test]
fn endless_recursion_overflows_the_stack_and_stops_the_app() {
    let runner = Runner::new().unwrap();
    let main = limit_app("m.set(\"\" + deep(1));").replace(
        "const m = signal",
        "function deep(n: number): number { return deep(n + 1) + 1; }\nconst m = signal",
    );
    let (guest, tree) = start_limited(&runner, &main, SHORT);
    assert_stops(&runner, guest, tree, "too many nested calls");
}

#[test]
fn endless_allocation_hits_the_memory_limit() {
    let runner = Runner::new().unwrap();
    let (guest, tree) = start_limited(
        &runner,
        &limit_app("const keep: string[][] = []; while (keep.length >= 0) { keep.push([\"aaaaaaaaaaaaaaaa\" + keep.length]); }"),
        SHORT,
    );
    // The guest fails to grow its memory: wasm `memory.grow` returns -1 and
    // the runtime traps (out of memory), before the time limit.
    let mut guest = guest;
    let mut tree = tree;
    let e = press(&mut guest, &mut tree, "go").expect_err("the call must fail");
    // The top message is the plain reason that the host shows.
    assert_eq!(e.to_string(), "out of memory", "{e:#}");
    let e = press(&mut guest, &mut tree, "ping").expect_err("a stopped app is not called again");
    assert!(format!("{e:#}").contains("stopped earlier"), "{e:#}");
}

// -- Cost --------------------------------------------------------------------------

#[test]
fn an_app_without_throw_does_not_need_core_1_10() {
    let a = compile(&show("", "const x = 1;", "\"\" + x"));
    assert!(!a.runtime.ends_with("1.10"), "runtime {}", a.runtime);
    let b = compile(&show("", "let m = \"\"; try { m = \"a\"; } catch (e) { m = e.message; }", "m"));
    assert!(!b.runtime.ends_with("1.10"), "a try with no throw reaches no new function: {}", b.runtime);
    let c = compile(HANDLER_SRC);
    assert!(c.runtime.ends_with("1.10"), "an uncaught throw needs `uncaught`: {}", c.runtime);
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
