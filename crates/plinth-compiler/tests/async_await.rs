//! `async`/`await` (SPEC.md §4.5, core 1.10) on top of the request and
//! `completion` machinery: host calls without a `done` callback return a
//! `Promise`, `await` resumes in a continuation, signals set after an
//! `await` re-render, a rejected `await` throws, and pending
//! continuations survive the GC.

use plinth_compiler::driver::{Frontend, MemFs, frontend};
use plinth_protocol::{ControlKind, Event, NodeId, Value, Writer, event};
use plinth_runner_wasmtime::{DialogKind, Guest, Limits, Runner};
use plinth_ui::tree::{Node, Tree};

fn render(f: &Frontend) -> String {
    f.diags.iter().map(|d| f.sources.render(d)).collect::<Vec<_>>().join("\n")
}

fn codes(main: &str) -> Vec<&'static str> {
    let fs = MemFs::default().with("app/main.tsx", main);
    let f = frontend(&fs);
    eprintln!("{}", render(&f));
    f.diags.iter().map(|d| d.code).collect()
}

fn start_with(main: &str, stress: bool) -> (Guest, Tree) {
    let fs = MemFs::default().with("app/main.tsx", main);
    let (front, artifact) = plinth_compiler::compile(&fs).expect("compile");
    let artifact = artifact.unwrap_or_else(|| panic!("compile errors:\n{}", render(&front)));
    let runner = Runner::new().unwrap();
    let mut guest = runner.load(&artifact.component, Limits::default()).unwrap();
    let mut tree = Tree::new();
    let args = if stress { plinth_protocol::init_arg::one(plinth_protocol::init_arg::GC_STRESS, &[]) } else { Vec::new() };
    let commits = guest.init(&args).unwrap();
    apply(&mut guest, &mut tree, commits);
    (guest, tree)
}

fn start(main: &str) -> (Guest, Tree) {
    start_with(main, false)
}

fn apply(guest: &mut Guest, tree: &mut Tree, commits: Vec<Vec<u8>>) {
    for log in guest.take_logs() {
        eprintln!("guest: {log}");
    }
    for commit in commits {
        let errors = tree.apply(&commit).unwrap();
        assert!(errors.is_empty(), "op errors: {errors:?}");
    }
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

fn press(guest: &mut Guest, tree: &mut Tree, label: &str) {
    let id = find(tree, ControlKind::Button, |n| n.str_prop(plinth_protocol::prop::LABEL) == Some(label));
    let handler = tree.get(id).and_then(|n| n.handler(event::PRESS)).expect("button has a handler");
    let mut w = Writer::new();
    w.event(&Event::Ui { handler, event: event::PRESS, value: Value::Null });
    let commits = guest.on_event(w.as_bytes()).unwrap();
    apply(guest, tree, commits);
}

/// Answers the one open dialog, checking its kind and message.
fn answer(guest: &mut Guest, tree: &mut Tree, kind: DialogKind, message: &str, result: Value) {
    let pending = guest.pending_dialogs().to_vec();
    assert_eq!(pending.len(), 1, "exactly one dialog should be open: {pending:?}");
    assert_eq!(pending[0].kind, kind);
    assert_eq!(pending[0].message, message);
    let commits = guest.answer_dialog(pending[0].id, result).unwrap();
    apply(guest, tree, commits);
}

/// A program with a `Text` that shows `status` and one `go` button whose
/// handler is `handler`; `top` holds top-level declarations.
fn app(top: &str, handler: &str) -> String {
    format!(
        r#"import {{ app, Screen, Text, Button, signal }} from "plinth:ui";
import {{ alert, confirm, prompt }} from "plinth:dialog";
const status = signal("idle");
{top}
function Home() {{
  return <Screen title="Home">
    <Text>{{status()}}</Text>
    <Button label="go" onPress={{{handler}}} />
  </Screen>;
}}
export default app({{ screens: {{ home: {{ title: "Home", component: Home }} }} }});
"#
    )
}

#[test]
fn await_confirm_in_a_handler_updates_a_signal() {
    let main = app("", r#"async () => { status.set("asking"); const ok = await confirm("Sure?"); status.set(ok ? "yes" : "no"); }"#);
    let (mut guest, mut tree) = start(&main);
    press(&mut guest, &mut tree, "go");
    assert_eq!(text(&tree), "asking");
    answer(&mut guest, &mut tree, DialogKind::Confirm, "Sure?", Value::Bool(true));
    assert_eq!(text(&tree), "yes", "a signal set after an await re-renders");
    assert!(guest.take_errors().is_empty());
}

#[test]
fn sequential_awaits_and_prompt_values() {
    let top = r#"async function ask(): Promise<string> {
  const name = await prompt("Name?");
  if (name === null) { return "nobody"; }
  const ok = await confirm(`Greet ${name}?`);
  return ok ? `hello ${name}` : "no greeting";
}"#;
    let main = app(top, r#"async () => { const r = await ask(); await alert(r); status.set("done: " + r); }"#);
    let (mut guest, mut tree) = start(&main);
    press(&mut guest, &mut tree, "go");
    answer(&mut guest, &mut tree, DialogKind::Prompt, "Name?", Value::Str("Ada".into()));
    answer(&mut guest, &mut tree, DialogKind::Confirm, "Greet Ada?", Value::Bool(true));
    assert_eq!(text(&tree), "idle");
    answer(&mut guest, &mut tree, DialogKind::Alert, "hello Ada", Value::Null);
    assert_eq!(text(&tree), "done: hello Ada");

    // The early `return` path.
    press(&mut guest, &mut tree, "go");
    answer(&mut guest, &mut tree, DialogKind::Prompt, "Name?", Value::Null);
    answer(&mut guest, &mut tree, DialogKind::Alert, "nobody", Value::Null);
    assert_eq!(text(&tree), "done: nobody");
}

#[test]
fn await_of_an_already_settled_promise_resumes() {
    let top = "async function two(): Promise<number> { return 2; }\nasync function four(): Promise<number> { const a = await two(); const b = await two(); return a + b; }";
    let main = app(top, r#"async () => { const n = await four(); status.set("n=" + n); }"#);
    let (mut guest, mut tree) = start(&main);
    press(&mut guest, &mut tree, "go");
    assert_eq!(text(&tree), "n=4");
}

#[test]
fn await_runs_after_the_synchronous_code_like_js() {
    let top = r#"let log = "";
async function f(): Promise<void> { log = log + "a"; await g(); log = log + "c"; }
async function g(): Promise<void> {}"#;
    let main = app(top, r#"() => { f(); log = log + "b"; status.set(log); }"#);
    let (mut guest, mut tree) = start(&main);
    press(&mut guest, &mut tree, "go");
    assert_eq!(text(&tree), "ab");
    press(&mut guest, &mut tree, "go");
    assert_eq!(text(&tree), "abcab");
}

#[test]
fn await_inside_if_else() {
    let top = r#"async function run(flag: boolean): Promise<string> {
  let s = "start";
  if (flag) {
    const ok = await confirm("flag?");
    s = s + (ok ? "+ok" : "+no");
  } else {
    s = s + "+skip";
  }
  s = s + "+end";
  return s;
}
let flag = false;"#;
    let main = app(top, r#"async () => { flag = !flag; status.set(await run(flag)); }"#);
    let (mut guest, mut tree) = start(&main);
    press(&mut guest, &mut tree, "go");
    answer(&mut guest, &mut tree, DialogKind::Confirm, "flag?", Value::Bool(false));
    assert_eq!(text(&tree), "start+no+end");
    press(&mut guest, &mut tree, "go");
    assert_eq!(text(&tree), "start+skip+end");
}

#[test]
fn await_inside_loops_with_break_and_continue() {
    let top = r#"async function collect(): Promise<string> {
  let out = "";
  for (const q of ["a", "b", "c", "d"]) {
    if (q === "b") { continue; }
    const ok = await confirm(q);
    if (!ok) { break; }
    out = out + q;
  }
  let i = 0;
  while (i < 3) {
    i = i + 1;
    if (i === 2) { continue; }
    const v = await prompt("n" + i);
    out = out + "|" + (v ?? "-");
  }
  for (let j = 0; j < 2; j++) { out = out + "/" + j; }
  return out;
}"#;
    let main = app(top, r#"async () => { status.set(await collect()); }"#);
    let (mut guest, mut tree) = start(&main);
    press(&mut guest, &mut tree, "go");
    answer(&mut guest, &mut tree, DialogKind::Confirm, "a", Value::Bool(true));
    answer(&mut guest, &mut tree, DialogKind::Confirm, "c", Value::Bool(true));
    answer(&mut guest, &mut tree, DialogKind::Confirm, "d", Value::Bool(false));
    answer(&mut guest, &mut tree, DialogKind::Prompt, "n1", Value::Str("x".into()));
    answer(&mut guest, &mut tree, DialogKind::Prompt, "n3", Value::Null);
    assert_eq!(text(&tree), "ac|x|-/0/1");
}

#[test]
fn a_loop_whose_iterations_do_not_await_stays_flat() {
    // 20,000 iterations that do not wait must not grow the Wasm stack.
    let top = r#"async function count(n: number): Promise<number> {
  let s = 0;
  for (let i = 0; i < n; i++) {
    if (i === n - 1) { await alert("last"); }
    s = s + 1;
  }
  return s;
}"#;
    let main = app(top, r#"async () => { status.set("" + await count(20000)); }"#);
    let (mut guest, mut tree) = start(&main);
    press(&mut guest, &mut tree, "go");
    answer(&mut guest, &mut tree, DialogKind::Alert, "last", Value::Null);
    assert_eq!(text(&tree), "20000");
}

#[test]
fn a_throw_after_await_rejects_and_the_awaiting_catch_gets_it() {
    let top = r#"class Cancelled extends Error { constructor() { super("cancelled"); this.name = "Cancelled"; } }
async function step(): Promise<number> {
  const ok = await confirm("go on?");
  if (!ok) { throw new Cancelled(); }
  return 7;
}"#;
    let handler = r#"async () => {
  try {
    const n = await step();
    status.set("got " + n);
  } catch (e) {
    status.set(e instanceof Cancelled ? "was cancelled" : "other");
  }
  status.set(status() + "!");
}"#;
    let main = app(top, handler);
    let (mut guest, mut tree) = start(&main);
    press(&mut guest, &mut tree, "go");
    answer(&mut guest, &mut tree, DialogKind::Confirm, "go on?", Value::Bool(false));
    assert_eq!(text(&tree), "was cancelled!");
    press(&mut guest, &mut tree, "go");
    answer(&mut guest, &mut tree, DialogKind::Confirm, "go on?", Value::Bool(true));
    assert_eq!(text(&tree), "got 7!");
    assert!(guest.take_errors().is_empty());
}

#[test]
fn await_inside_a_catch_block() {
    let top = r#"async function risky(): Promise<number> { await alert("try"); throw new Error("bad"); }"#;
    let handler = r#"async () => {
  try { await risky(); status.set("no"); }
  catch (e) { const ok = await confirm("retry after " + e.message + "?"); status.set(ok ? "retry" : "give up"); }
}"#;
    let main = app(top, handler);
    let (mut guest, mut tree) = start(&main);
    press(&mut guest, &mut tree, "go");
    answer(&mut guest, &mut tree, DialogKind::Alert, "try", Value::Null);
    answer(&mut guest, &mut tree, DialogKind::Confirm, "retry after bad?", Value::Bool(true));
    assert_eq!(text(&tree), "retry");
}

#[test]
fn an_unawaited_rejection_is_reported_and_the_app_keeps_running() {
    let main = app("", r#"async () => { const ok = await confirm("fail?"); if (ok) { throw new Error("async boom"); } status.set("fine"); }"#);
    let (mut guest, mut tree) = start(&main);
    press(&mut guest, &mut tree, "go");
    answer(&mut guest, &mut tree, DialogKind::Confirm, "fail?", Value::Bool(true));
    assert_eq!(guest.take_errors(), vec!["Uncaught (in promise) Error: async boom".to_string()]);
    press(&mut guest, &mut tree, "go");
    answer(&mut guest, &mut tree, DialogKind::Confirm, "fail?", Value::Bool(false));
    assert_eq!(text(&tree), "fine");
    assert!(guest.take_errors().is_empty());
}

#[test]
fn a_rejection_awaited_later_in_the_same_event_is_not_reported() {
    let top = r#"async function bad(): Promise<number> { throw new Error("x"); }"#;
    let main = app(top, r#"async () => { const p = bad(); try { await p; } catch (e) { status.set("caught " + e.message); } }"#);
    let (mut guest, mut tree) = start(&main);
    press(&mut guest, &mut tree, "go");
    assert_eq!(text(&tree), "caught x");
    assert!(guest.take_errors().is_empty());
}

#[test]
fn pending_continuations_survive_gc_stress() {
    let top = r#"async function chain(): Promise<string> {
  const parts: string[] = [];
  for (const q of ["one", "two", "three"]) {
    const v = await prompt(q);
    parts.push((v ?? "?") + "-" + q);
  }
  return parts.join(",");
}"#;
    let main = app(top, r#"async () => { status.set("[" + await chain() + "]"); }"#);
    let (mut guest, mut tree) = start_with(&main, true);
    press(&mut guest, &mut tree, "go");
    // Every on-event collects in stress mode, so the continuations and the
    // `parts` array live only through the pending requests.
    answer(&mut guest, &mut tree, DialogKind::Prompt, "one", Value::Str("a".repeat(40)));
    answer(&mut guest, &mut tree, DialogKind::Prompt, "two", Value::Str("b".into()));
    answer(&mut guest, &mut tree, DialogKind::Prompt, "three", Value::Str("c".into()));
    assert_eq!(text(&tree), format!("[{}-one,b-two,c-three]", "a".repeat(40)));
}

#[test]
fn a_detached_async_callback_where_void_is_expected() {
    let top = r#"function each(xs: string[], f: (x: string) => void): void { for (const x of xs) { f(x); } }"#;
    let main = app(top, r#"() => { each(["p", "q"], async (x) => { const ok = await confirm(x); if (ok) { status.set(status() + x); } }); }"#);
    let (mut guest, mut tree) = start(&main);
    press(&mut guest, &mut tree, "go");
    let pending = guest.pending_dialogs().to_vec();
    assert_eq!(pending.len(), 2);
    for d in pending {
        let commits = guest.answer_dialog(d.id, Value::Bool(true)).unwrap();
        apply(&mut guest, &mut tree, commits);
    }
    assert_eq!(text(&tree), "idlepq");
}

#[test]
fn the_callback_forms_still_work_and_need_no_new_core() {
    let main = app("", r#"() => confirm("cb?", (ok) => { status.set(ok ? "y" : "n"); })"#);
    let fs = MemFs::default().with("app/main.tsx", &main);
    let (_, artifact) = plinth_compiler::compile(&fs).unwrap();
    assert!(!artifact.unwrap().runtime.ends_with("1.10"));
}

// -- Diagnostics -------------------------------------------------------------------

#[test]
fn await_outside_an_async_function_is_rejected() {
    // Inside a function that is not `async`, `await` is a syntax error
    // (PL1000); at the top level of a module it is PL2009.
    assert_eq!(codes(&app("", r#"() => { const ok = await confirm("x"); }"#)), vec!["PL1000"]);
    assert_eq!(codes(&app(r#"const first = await confirm("x");"#, "() => {}")), vec!["PL2009"]);
}

#[test]
fn await_inside_expressions_keeps_the_order_of_evaluation() {
    let top = r#"let log = "";
function note(s: string): string { log = log + s; return s; }
async function later(s: string): Promise<string> { await alert(s); log = log + "!"; return s.toUpperCase(); }"#;
    let handler = r#"async () => {
  const a = note("a") + (await later("b")) + note("c");
  const flag = false;
  const b = flag && (await confirm("never")) ? "x" : "y";
  const c = (await confirm("c?")) ? await later("d") : "no";
  status.set(a + "|" + b + "|" + c + "|" + log);
}"#;
    let main = app(top, handler);
    let (mut guest, mut tree) = start(&main);
    press(&mut guest, &mut tree, "go");
    answer(&mut guest, &mut tree, DialogKind::Alert, "b", Value::Null);
    answer(&mut guest, &mut tree, DialogKind::Confirm, "c?", Value::Bool(true));
    answer(&mut guest, &mut tree, DialogKind::Alert, "d", Value::Null);
    assert_eq!(text(&tree), "aBc|y|D|a!c!");
}

#[test]
fn await_inside_try_finally_is_rejected() {
    assert_eq!(codes(&app("", r#"async () => { try { await alert("x"); } finally { status.set("f"); } }"#)), vec!["PL2009"]);
}

#[test]
fn await_of_a_non_promise_is_rejected() {
    assert_eq!(codes(&app("", r#"async () => { const n = await 5; }"#)), vec!["PL3001"]);
}

#[test]
fn an_async_function_must_return_a_promise_type() {
    assert_eq!(codes(&app("async function f(): number { return 1; }", "() => {}")), vec!["PL3001"]);
}

/// `examples/dialogs` uses `async`/`await` for its handlers.
#[test]
fn the_dialogs_example_greets_with_three_awaits() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/dialogs");
    let fs = plinth_compiler::driver::DiskFs { root };
    let (front, artifact) = plinth_compiler::compile(&fs).expect("compile");
    let artifact = artifact.unwrap_or_else(|| panic!("compile errors:\n{}", render(&front)));
    let runner = Runner::new().unwrap();
    let mut guest = runner.load(&artifact.component, Limits::default()).unwrap();
    let mut tree = Tree::new();
    let commits = guest.init(&[]).unwrap();
    apply(&mut guest, &mut tree, commits);
    press(&mut guest, &mut tree, "Greet");
    answer(&mut guest, &mut tree, DialogKind::Prompt, "Who do you want to greet?", Value::Str("Grace".into()));
    answer(&mut guest, &mut tree, DialogKind::Confirm, "Greet Grace?", Value::Bool(true));
    answer(&mut guest, &mut tree, DialogKind::Alert, "Hello, Grace!", Value::Null);
    let name = find(&tree, ControlKind::Text, |n| n.text.as_deref() == Some("Grace"));
    assert!(tree.get(name).is_some());
    press(&mut guest, &mut tree, "Confirm");
    answer(&mut guest, &mut tree, DialogKind::Confirm, "Reset the count?", Value::Bool(true));
    assert!(guest.take_errors().is_empty());
}

#[test]
fn hub_and_net_calls_have_promise_forms() {
    let main = r#"import { app, Screen, Text, Button, signal } from "plinth:ui";
import { search, install } from "plinth:hub";
import { fetch } from "plinth:net";
const out = signal("");
async function go(): Promise<void> {
  const hits = await search("notes");
  const err = await install("x");
  const r = await fetch("https://example.com/", null);
  out.set((hits ?? "") + (err ?? "") + r.status);
}
function Home() { return <Screen title="Home"><Text>{out()}</Text><Button label="go" onPress={() => { go(); }} /></Screen>; }
export default app({ screens: { home: { title: "Home", component: Home } } });
"#;
    let fs = MemFs::default().with("app/main.tsx", main);
    let caps = vec!["hub.manage".to_string(), "net:example.com".to_string()];
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &caps).expect("compile");
    assert!(artifact.is_some(), "{}", render(&front));
}
