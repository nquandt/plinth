//! `plinth:secrets` (core 1.13, `docs/SECRETS.md`) through a real core. The
//! tests use hand-written app code (as in `notify.rs`) that calls
//! `secrets_set`, `secrets_get`, `secrets_remove`, `secrets_names` and
//! `secrets_last_error`. The runner answers through the policy and a
//! `MemorySecrets`. Set `PLINTH_ENGINE=wasmi` to run them on wasmi.

use plinth_compiler::link::{self, AppCode, Layout};
use plinth_protocol::Value;
use plinth_runner_wasmtime::policy::Policy;
use plinth_runner_wasmtime::secrets::{MemorySecrets, SECRETS, SecretsBackend, SecretsScope};
use plinth_runner_wasmtime::{Guest, Limits, MemoryClipboard, Runner, kv::Kv};
use wasm_encoder::{BlockType, Function, Instruction as I, ValType};

/// One call that `main` makes.
#[derive(Clone)]
enum Op {
    Set(String, String),
    Get(String),
    Remove(String),
    Names,
}

fn set(name: &str, value: &str) -> Op {
    Op::Set(name.to_owned(), value.to_owned())
}

fn get(name: &str) -> Op {
    Op::Get(name.to_owned())
}

fn remove(name: &str) -> Op {
    Op::Remove(name.to_owned())
}

/// Adds `text` as a data segment and emits code that makes a heap string of
/// it and stores the pointer in `local`.
fn string(app: &mut AppCode, layout: &Layout, f: &mut Function, text: &str, local: u32) {
    let seg = layout.data_count + app.data.len() as u32;
    app.data.push(text.as_bytes().to_vec());
    f.instruction(&I::I32Const(text.len() as i32))
        .instruction(&I::Call(layout.rt("str_new")))
        .instruction(&I::LocalTee(local))
        .instruction(&I::I32Const(12))
        .instruction(&I::I32Add)
        .instruction(&I::I32Const(0))
        .instruction(&I::I32Const(text.len() as i32))
        .instruction(&I::MemoryInit { mem: 0, data_index: seg });
}

/// Logs the string in `local` if it is not null.
fn log_if_set(layout: &Layout, f: &mut Function, local: u32) {
    f.instruction(&I::LocalGet(local))
        .instruction(&I::If(BlockType::Empty))
        .instruction(&I::LocalGet(local))
        .instruction(&I::Call(layout.rt("log")))
        .instruction(&I::End);
}

/// An app whose `main` makes the calls of `ops` in order. The callback of
/// each call logs the `error` of the completion and `secrets_last_error()`,
/// but never the `text` (it can hold a value).
fn secrets_app(ops: &[Op]) -> Vec<u8> {
    let rt = link::runtime();
    let layout = link::layout(rt).expect("layout");
    let rtf = |n: &str| layout.rt(n);

    let mut app = AppCode::default();
    let thunk_ty = layout.type_count; // (i32) -> ()
    let void_ty = layout.type_count + 1; // () -> ()
    app.types.push((vec![ValType::I32], vec![]));
    app.types.push((vec![], vec![]));
    let main_idx = layout.func_count;
    let callback_idx = layout.func_count + 1;
    let start_idx = layout.func_count + 2;
    let callback_slot = layout.table_size as i32 + 1;

    // main(env): local 0 is the env; locals 1 and 2 hold the strings of one
    // call at a time.
    let mut main = Function::new(vec![(2, ValType::I32)]);
    for op in ops {
        main.instruction(&I::I32Const(callback_slot)).instruction(&I::I32Const(0));
        match op {
            Op::Set(name, value) => {
                string(&mut app, &layout, &mut main, name, 1);
                string(&mut app, &layout, &mut main, value, 2);
                main.instruction(&I::LocalGet(1)).instruction(&I::LocalGet(2)).instruction(&I::Call(rtf("secrets_set")));
            }
            Op::Get(name) => {
                string(&mut app, &layout, &mut main, name, 1);
                main.instruction(&I::LocalGet(1)).instruction(&I::Call(rtf("secrets_get")));
            }
            Op::Remove(name) => {
                string(&mut app, &layout, &mut main, name, 1);
                main.instruction(&I::LocalGet(1)).instruction(&I::Call(rtf("secrets_remove")));
            }
            Op::Names => {
                main.instruction(&I::Call(rtf("secrets_names")));
            }
        }
    }
    main.instruction(&I::End);
    app.funcs.push((thunk_ty, main));
    app.table.push(main_idx);

    // callback(env): locals 1 error, 2 last error.
    let mut callback = Function::new(vec![(2, ValType::I32)]);
    callback
        .instruction(&I::Call(rtf("net_result_error")))
        .instruction(&I::LocalSet(1))
        .instruction(&I::Call(rtf("secrets_last_error")))
        .instruction(&I::LocalSet(2));
    log_if_set(&layout, &mut callback, 1);
    log_if_set(&layout, &mut callback, 2);
    callback.instruction(&I::End);
    app.funcs.push((thunk_ty, callback));
    app.table.push(callback_idx);

    let mut start = Function::new(vec![]);
    if let Some(s) = layout.rt_start {
        start.instruction(&I::Call(s));
    }
    start
        .instruction(&I::I32Const(layout.table_size as i32))
        .instruction(&I::Call(rtf("set_main")))
        .instruction(&I::End);
    app.funcs.push((void_ty, start));
    app.start = Some(start_idx);

    let core = link::link(rt, &layout, &app).expect("link");
    link::componentize(&core).expect("componentize")
}

/// What one run gives back.
struct Run {
    /// The completion results in call order.
    results: Vec<Value>,
    /// The logs of `init` and of the callbacks.
    logs: Vec<String>,
}

/// Runs the app of `ops` with `policy`. `store` is the backend and the id of
/// the app; `None` keeps the default backend of the runner.
fn run(ops: &[Op], policy: Policy, store: Option<(&MemorySecrets, &str)>) -> Run {
    let component = secrets_app(ops);
    let runner = Runner::new().unwrap();
    let mut guest: Guest = runner
        .load_with_policy(&component, Limits::default(), policy, Kv::in_memory(), Box::new(MemoryClipboard::default()))
        .unwrap();
    if let Some((backend, app_id)) = store {
        guest.set_secrets(Box::new(backend.clone()), app_id);
    }
    guest.init(&[]).unwrap();
    let mut logs = guest.take_logs();
    let results = guest.poll_net_results();
    assert_eq!(results.len(), ops.len(), "one completion for each call");
    for (id, result) in &results {
        guest.answer_dialog(*id, result.clone()).unwrap();
    }
    logs.extend(guest.take_logs());
    Run { results: results.into_iter().map(|(_, v)| v).collect(), logs }
}

fn ok(n: i32, text: &str) -> Value {
    Value::List(vec![Value::Bool(true), Value::Int(n), Value::Str(text.into()), Value::Null])
}

fn denied(error: &str) -> Value {
    Value::List(vec![Value::Bool(false), Value::Int(0), Value::Str(String::new()), Value::Str(error.into())])
}

#[test]
fn round_trip() {
    let backend = MemorySecrets::new();
    let ops = [set("api-token", "abc123"), set("other", "x"), get("api-token"), Op::Names, remove("api-token"), get("api-token")];
    let r = run(&ops, Policy::new([SECRETS]), Some((&backend, "app.one")));
    assert_eq!(
        r.results,
        vec![ok(0, ""), ok(0, ""), ok(1, "abc123"), ok(2, r#"["api-token","other"]"#), ok(0, ""), ok(0, "")]
    );
    assert!(r.logs.is_empty(), "no call was denied: {:?}", r.logs);
}

#[test]
fn apps_do_not_see_each_other() {
    let backend = MemorySecrets::new();
    let a = run(&[set("t", "one"), get("t")], Policy::new([SECRETS]), Some((&backend, "app.a")));
    assert_eq!(a.results, vec![ok(0, ""), ok(1, "one")]);
    let b = run(&[get("t"), Op::Names, set("t", "two"), get("t")], Policy::new([SECRETS]), Some((&backend, "app.b")));
    assert_eq!(b.results, vec![ok(0, ""), ok(0, "[]"), ok(0, ""), ok(1, "two")]);
    // A new guest of app a still sees its own value, and app b removes only its own.
    let a2 = run(&[get("t")], Policy::new([SECRETS]), Some((&backend, "app.a")));
    assert_eq!(a2.results, vec![ok(1, "one")]);
    run(&[remove("t")], Policy::new([SECRETS]), Some((&backend, "app.b")));
    let mut direct = backend.clone();
    assert_eq!(direct.get(&SecretsScope::new("app.a"), "t"), Ok(Some("one".to_owned())));
    assert_eq!(direct.get(&SecretsScope::new("app.b"), "t"), Ok(None));
}

#[test]
fn limits_complete_with_denied_limit() {
    let backend = MemorySecrets::new();
    let big = "x".repeat(16 * 1024);
    let ops = [
        set("", "v"),
        set("has space", "v"),
        set(&"a".repeat(65), "v"),
        set("big", &big),
        set("too-big", &format!("{big}x")),
        get("bad/name"),
        get("big"),
    ];
    let r = run(&ops, Policy::new([SECRETS]), Some((&backend, "app.one")));
    let limit = denied("denied:limit");
    assert_eq!(r.results[..3], [limit.clone(), limit.clone(), limit.clone()]);
    assert_eq!(r.results[3], ok(0, ""));
    assert_eq!(r.results[4], limit);
    assert_eq!(r.results[5], limit);
    assert_eq!(r.results[6], ok(1, &big));
    // Each denied call sets `lastError()`; the callbacks log the error and the last error.
    assert_eq!(r.logs.iter().filter(|l| *l == "denied:limit").count(), 10);
}

#[test]
fn the_65th_secret_is_denied() {
    let backend = MemorySecrets::new();
    let mut ops: Vec<Op> = (0..64).map(|i| set(&format!("s{i}"), "v")).collect();
    ops.push(set("extra", "v"));
    ops.push(set("s0", "new"));
    ops.push(get("s0"));
    let r = run(&ops, Policy::new([SECRETS]), Some((&backend, "app.one")));
    assert!(r.results[..64].iter().all(|v| *v == ok(0, "")));
    assert_eq!(r.results[64], denied("denied:limit"));
    assert_eq!(r.results[65], ok(0, ""));
    assert_eq!(r.results[66], ok(1, "new"));
}

#[test]
fn undeclared_calls_are_denied_without_a_trap() {
    let backend = MemorySecrets::new();
    let ops = [set("t", "topsecret"), get("t"), remove("t"), Op::Names];
    let r = run(&ops, Policy::new(["store.kv"]), Some((&backend, "app.one")));
    assert_eq!(r.results, vec![denied("denied:undeclared"); 4]);
    assert_eq!(r.logs, ["denied:undeclared", "denied:undeclared"].repeat(4));
    let mut direct = backend.clone();
    assert_eq!(direct.names(&SecretsScope::new("app.one")), Ok(vec![]), "no call reached the backend");
}

#[test]
fn refused_calls_are_denied_without_a_trap() {
    let backend = MemorySecrets::new();
    let mut policy = Policy::new([SECRETS]);
    policy.refuse(SECRETS);
    let r = run(&[set("t", "topsecret"), get("t")], policy, Some((&backend, "app.one")));
    assert_eq!(r.results, vec![denied("denied:refused"); 2]);
    assert_eq!(r.logs, ["denied:refused", "denied:refused"].repeat(2));
    let mut direct = backend.clone();
    assert_eq!(direct.names(&SecretsScope::new("app.one")), Ok(vec![]));
}

#[test]
fn a_host_with_no_safe_store_answers_unsupported() {
    // No `set_secrets`: the default backend. It never falls back to a file.
    let r = run(&[set("t", "topsecret"), get("t"), Op::Names], Policy::new([SECRETS]), None);
    assert_eq!(r.results, vec![denied("denied:unsupported"); 3]);
    assert_eq!(r.logs, ["denied:unsupported", "denied:unsupported"].repeat(3));
}

#[test]
fn a_value_is_never_in_a_log() {
    let backend = MemorySecrets::new();
    let r = run(&[set("t", "topsecret"), get("t"), set("", "topsecret")], Policy::new([SECRETS]), Some((&backend, "app.one")));
    assert!(r.logs.iter().all(|l| !l.contains("topsecret")), "{:?}", r.logs);
}

// -- Compiled apps -------------------------------------------------------------

/// TypeScript apps that import `plinth:secrets`, compiled by the compiler.
mod compiled {
    use super::*;
    use plinth_compiler::driver::MemFs;
    use plinth_protocol::{ControlKind, Event, NodeId, Writer, event};
    use plinth_ui::tree::{Node, Tree};
    use std::time::{Duration, Instant};

    /// An app whose button runs `body` in `async function go()`; `body` sets
    /// `result` to a text that starts with `done:` at the end.
    fn app_src(body: &str) -> String {
        format!(
            r#"import {{ app, Screen, Text, Button, signal }} from "plinth:ui";
import {{ set, get, remove, names, lastError }} from "plinth:secrets";
const result = signal("waiting");
async function go(): Promise<void> {{
{body}
}}
function Home() {{
  return <Screen title="Home">
    <Text>{{result()}}</Text>
    <Button label="go" onPress={{() => {{ go(); }}}} />
  </Screen>;
}}
export default app({{ screens: {{ home: {{ title: "Home", component: Home }} }} }});
"#
        )
    }

    struct App {
        guest: Guest,
        tree: Tree,
    }

    fn start(body: &str, policy: Policy, store: Option<(&MemorySecrets, &str)>) -> App {
        let fs = MemFs::default().with("app/main.tsx", &app_src(body));
        let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[SECRETS.to_owned()]).expect("compile");
        let artifact = artifact.unwrap_or_else(|| {
            let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
            panic!("compile errors:\n{}", diags.join("\n"))
        });
        let runner = Runner::new().unwrap();
        let mut guest = runner
            .load_with_policy(&artifact.component, Limits::default(), policy, Kv::in_memory(), Box::new(MemoryClipboard::default()))
            .unwrap();
        if let Some((backend, app_id)) = store {
            guest.set_secrets(Box::new(backend.clone()), app_id);
        }
        let mut tree = Tree::new();
        for commit in guest.init(&[]).unwrap() {
            assert!(tree.apply(&commit).unwrap().is_empty());
        }
        App { guest, tree }
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

    impl App {
        /// Presses the button, then delivers completions until the text
        /// starts with `done:`, and returns the text after it.
        fn go(&mut self) -> String {
            let button = find(&self.tree, ControlKind::Button, |_| true);
            let handler = self.tree.get(button).and_then(|n| n.handler(event::PRESS)).unwrap();
            let mut w = Writer::new();
            w.event(&Event::Ui { handler, event: event::PRESS, value: Value::Null });
            for commit in self.guest.on_event(w.as_bytes()).unwrap() {
                assert!(self.tree.apply(&commit).unwrap().is_empty());
            }
            let start = Instant::now();
            loop {
                if let Some(rest) = text(&self.tree).strip_prefix("done:") {
                    assert!(self.guest.take_errors().is_empty(), "uncaught errors");
                    return rest.to_owned();
                }
                for (id, result) in self.guest.poll_net_results() {
                    for commit in self.guest.answer_dialog(id, result).unwrap() {
                        assert!(self.tree.apply(&commit).unwrap().is_empty());
                    }
                }
                assert!(start.elapsed() < Duration::from_secs(10), "no result; text is {:?}", text(&self.tree));
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }

    fn run(body: &str, backend: &MemorySecrets, app_id: &str) -> String {
        start(body, Policy::new([SECRETS]), Some((backend, app_id))).go()
    }

    #[test]
    fn compiled_round_trip() {
        let backend = MemorySecrets::new();
        let out = run(
            r#"
  await set("api-token", "abc123");
  const a = await get("api-token");
  const miss = await get("nothing");
  const n1 = await names();
  await remove("api-token");
  const b = await get("api-token");
  const n2 = await names();
  result.set("done:" + (a ?? "null") + ";" + (miss === null ? "null" : "value") + ";" + n1.join(",") + ";" + (b === null ? "null" : "value") + ";" + n2.length + ";" + (lastError() ?? "null"));
"#,
            &backend,
            "app.one",
        );
        assert_eq!(out, "abc123;null;api-token;null;0;null");
    }

    #[test]
    fn compiled_get_of_a_missing_name_gives_null_and_an_empty_value_is_not_null() {
        let backend = MemorySecrets::new();
        let out = run(
            r#"
  const a = await get("nope");
  await set("empty", "");
  const b = await get("empty");
  result.set("done:" + (a === null ? "null" : "value") + ";" + (b === null ? "null" : "[" + b + "]"));
"#,
            &backend,
            "app.one",
        );
        assert_eq!(out, "null;[]");
    }

    #[test]
    fn compiled_names_are_sorted() {
        let backend = MemorySecrets::new();
        let out = run(
            r#"
  await set("zeta", "1");
  await set("alpha", "2");
  await set("mid.dle", "3");
  const all = await names();
  result.set("done:" + all.length + ":" + all.join(","));
"#,
            &backend,
            "app.one",
        );
        assert_eq!(out, "3:alpha,mid.dle,zeta");
    }

    #[test]
    fn compiled_apps_do_not_see_each_other() {
        let backend = MemorySecrets::new();
        let a = run(r#"await set("t", "one"); result.set("done:" + ((await get("t")) ?? "null"));"#, &backend, "app.a");
        assert_eq!(a, "one");
        let b = run(
            r#"
  const before = await get("t");
  const list = await names();
  await set("t", "two");
  result.set("done:" + (before === null ? "null" : "value") + ";" + list.length + ";" + ((await get("t")) ?? "null"));
"#,
            &backend,
            "app.b",
        );
        assert_eq!(b, "null;0;two");
        let a2 = run(r#"result.set("done:" + ((await get("t")) ?? "null"));"#, &backend, "app.a");
        assert_eq!(a2, "one");
    }

    #[test]
    fn compiled_limit_errors_reject_with_denied_limit() {
        let backend = MemorySecrets::new();
        let out = run(
            r#"
  let r = "";
  try { await set("bad name", "x"); r = r + "ok;"; } catch (e) { r = r + e.message + ";"; }
  const first = lastError() ?? "null";
  try { await set("fine", "x".repeat(16385)); r = r + "ok;"; } catch (e) { r = r + e.message + ";"; }
  await set("fine", "x");
  const after = lastError() ?? "null";
  result.set("done:" + r + first + ";" + after);
"#,
            &backend,
            "app.one",
        );
        assert_eq!(out, "denied:limit;denied:limit;denied:limit;null");
    }

    #[test]
    fn compiled_denied_calls_reject_and_never_trap() {
        let body = r#"
  let r = "";
  try { await set("t", "topsecret"); r = r + "ok;"; } catch (e) { r = r + e.message + ";"; }
  try { const v = await get("t"); r = r + (v === null ? "null" : "value") + ";"; } catch (e) { r = r + e.message + ";"; }
  try { const l = await names(); r = r + l.length + ";"; } catch (e) { r = r + e.message + ";"; }
  result.set("done:" + r + (lastError() ?? "null"));
"#;
        let backend = MemorySecrets::new();
        let mut refused = Policy::new([SECRETS]);
        refused.refuse(SECRETS);
        let out = start(body, refused, Some((&backend, "app.one"))).go();
        assert_eq!(out, "denied:refused;denied:refused;denied:refused;denied:refused");

        let out = start(body, Policy::new(["store.kv"]), Some((&backend, "app.one"))).go();
        assert_eq!(out, "denied:undeclared;denied:undeclared;denied:undeclared;denied:undeclared");

        // No safe store: the default backend. It never falls back to a file.
        let out = start(body, Policy::new([SECRETS]), None).go();
        assert_eq!(out, "denied:unsupported;denied:unsupported;denied:unsupported;denied:unsupported");

        let mut direct = backend.clone();
        assert_eq!(direct.names(&SecretsScope::new("app.one")), Ok(vec![]), "no denied call reached the backend");
    }
}
