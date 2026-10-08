//! `plinth:notify` (core 1.12, `docs/PRODUCTION.md` H3) through a real core.
//! The first tests use hand-written app code (as in `link_spike.rs`) that
//! calls `notify_show`, `notify_cancel` and `notify_last_error`; the
//! `compiled_*` tests compile TypeScript that imports `plinth:notify`. The
//! runner answers through the policy and a `MemoryNotify`. Set
//! `PLINTH_ENGINE=wasmi` to run them on wasmi.

use plinth_compiler::link::{self, AppCode, Layout};
use plinth_protocol::Value;
use plinth_runner_wasmtime::notify::{MemoryNotify, NOTIFY, NotifyCall, NotifyOutcome};
use plinth_runner_wasmtime::policy::Policy;
use plinth_runner_wasmtime::{Guest, Limits, MemoryClipboard, Runner, kv::Kv};
use wasm_encoder::{BlockType, Function, Instruction as I, ValType};

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

/// An app whose `main` calls `show("Done", "3 files", "job")`, `show("Done")`
/// and `cancel("job")`, then logs `notify_last_error()` if it is set. The
/// callback of each `show` logs the completion's `text` and `error`.
fn notify_app() -> Vec<u8> {
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

    // main(env): locals 1 title, 2 body, 3 tag, 4 last error.
    let mut main = Function::new(vec![(4, ValType::I32)]);
    string(&mut app, &layout, &mut main, "Done", 1);
    string(&mut app, &layout, &mut main, "3 files", 2);
    string(&mut app, &layout, &mut main, "job", 3);
    main.instruction(&I::I32Const(callback_slot))
        .instruction(&I::I32Const(0))
        .instruction(&I::LocalGet(1))
        .instruction(&I::LocalGet(2))
        .instruction(&I::LocalGet(3))
        .instruction(&I::Call(rtf("notify_show")))
        .instruction(&I::I32Const(callback_slot))
        .instruction(&I::I32Const(0))
        .instruction(&I::LocalGet(1))
        .instruction(&I::I32Const(0))
        .instruction(&I::I32Const(0))
        .instruction(&I::Call(rtf("notify_show")))
        .instruction(&I::LocalGet(3))
        .instruction(&I::Call(rtf("notify_cancel")))
        .instruction(&I::Call(rtf("notify_last_error")))
        .instruction(&I::LocalSet(4));
    log_if_set(&layout, &mut main, 4);
    main.instruction(&I::End);
    app.funcs.push((thunk_ty, main));
    app.table.push(main_idx);

    // callback(env): locals 1 text, 2 error.
    let mut callback = Function::new(vec![(2, ValType::I32)]);
    callback
        .instruction(&I::Call(rtf("net_result_text")))
        .instruction(&I::LocalSet(1))
        .instruction(&I::Call(rtf("net_result_error")))
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

/// Runs the app with `policy` and `backend`: returns the logs of `init`, the
/// completion results in order, the logs of the callbacks, and the calls
/// that reached the backend.
fn run(policy: Policy, backend: MemoryNotify) -> (Vec<String>, Vec<Value>, Vec<String>, Vec<NotifyCall>) {
    let component = notify_app();
    let runner = Runner::new().unwrap();
    let mut guest: Guest = runner
        .load_with_policy(&component, Limits::default(), policy, Kv::in_memory(), Box::new(MemoryClipboard::default()))
        .unwrap();
    let calls = backend.calls();
    guest.set_notify(Box::new(backend));
    guest.init(&[]).unwrap();
    let init_logs = guest.take_logs();
    let results = guest.poll_net_results();
    assert_eq!(results.len(), 2, "one completion for each show");
    for (id, result) in &results {
        guest.answer_dialog(*id, result.clone()).unwrap();
    }
    let callback_logs = guest.take_logs();
    let calls = calls.lock().unwrap().clone();
    (init_logs, results.into_iter().map(|(_, v)| v).collect(), callback_logs, calls)
}

fn result(ok: bool, text: &str, error: Option<&str>) -> Value {
    Value::List(vec![
        Value::Bool(ok),
        Value::Int(0),
        Value::Str(text.into()),
        error.map_or(Value::Null, |e| Value::Str(e.into())),
    ])
}

#[test]
fn allowed_calls_reach_the_backend() {
    let (init_logs, results, callback_logs, calls) = run(Policy::new([NOTIFY]), MemoryNotify::default());
    assert!(init_logs.is_empty(), "cancel was not denied: {init_logs:?}");
    assert_eq!(results, vec![result(true, "shown", None); 2]);
    assert_eq!(callback_logs, ["shown", "shown"]);
    assert_eq!(
        calls,
        vec![
            NotifyCall::Show { title: "Done".into(), body: Some("3 files".into()), tag: Some("job".into()) },
            NotifyCall::Show { title: "Done".into(), body: None, tag: None },
            NotifyCall::Cancel { tag: "job".into() },
        ]
    );
}

#[test]
fn a_host_with_no_notification_area_answers_unsupported() {
    let (_, results, callback_logs, _) = run(Policy::new([NOTIFY]), MemoryNotify::new(NotifyOutcome::Unsupported));
    assert_eq!(results, vec![result(true, "unsupported", None); 2]);
    assert_eq!(callback_logs, ["unsupported", "unsupported"]);
}

#[test]
fn undeclared_calls_are_denied_without_a_trap() {
    let (init_logs, results, callback_logs, calls) = run(Policy::new(["store.kv"]), MemoryNotify::default());
    assert_eq!(init_logs, ["denied:undeclared"]);
    assert_eq!(results, vec![result(false, "", Some("denied:undeclared")); 2]);
    assert_eq!(callback_logs, ["", "denied:undeclared", "", "denied:undeclared"]);
    assert!(calls.is_empty());
}

#[test]
fn refused_calls_are_denied_without_a_trap() {
    let mut policy = Policy::new([NOTIFY]);
    policy.refuse(NOTIFY);
    let (init_logs, results, _, calls) = run(policy, MemoryNotify::default());
    assert_eq!(init_logs, ["denied:refused"]);
    assert_eq!(results, vec![result(false, "", Some("denied:refused")); 2]);
    assert!(calls.is_empty());
}

// -- Compiled apps -------------------------------------------------------------

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
import {{ show, cancel, lastError }} from "plinth:notify";
import type {{ NotifyResult }} from "plinth:notify";
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

    fn start(body: &str, policy: Policy, backend: MemoryNotify) -> App {
        let fs = MemFs::default().with("app/main.tsx", &app_src(body));
        let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[NOTIFY.to_owned()]).expect("compile");
        let artifact = artifact.unwrap_or_else(|| {
            let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
            panic!("compile errors:\n{}", diags.join("\n"))
        });
        let runner = Runner::new().unwrap();
        let mut guest = runner
            .load_with_policy(&artifact.component, Limits::default(), policy, Kv::in_memory(), Box::new(MemoryClipboard::default()))
            .unwrap();
        guest.set_notify(Box::new(backend));
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

    const BODY: &str = r#"
  let r = "";
  try {
    const a: NotifyResult = await show({ title: "Done", body: "3 files", tag: "job" });
    r = r + a + ";";
  } catch (e) {
    r = r + "err:" + e.message + ";";
  }
  try {
    const b = await show({ title: "Hi" });
    r = r + b + ";";
  } catch (e) {
    r = r + "err:" + e.message + ";";
  }
  cancel("job");
  r = r + (lastError() ?? "null");
  result.set("done:" + r);
"#;

    #[test]
    fn compiled_allowed_calls_reach_the_backend() {
        let backend = MemoryNotify::default();
        let calls = backend.calls();
        let mut app = start(BODY, Policy::new([NOTIFY]), backend);
        assert_eq!(app.go(), "shown;shown;null");
        assert_eq!(
            *calls.lock().unwrap(),
            vec![
                NotifyCall::Show { title: "Done".into(), body: Some("3 files".into()), tag: Some("job".into()) },
                NotifyCall::Show { title: "Hi".into(), body: None, tag: None },
                NotifyCall::Cancel { tag: "job".into() },
            ]
        );
    }

    #[test]
    fn compiled_unsupported_host_resolves_unsupported() {
        let mut app = start(BODY, Policy::new([NOTIFY]), MemoryNotify::new(NotifyOutcome::Unsupported));
        assert_eq!(app.go(), "unsupported;unsupported;null");
    }

    #[test]
    fn compiled_denied_calls_reject_and_never_trap() {
        let backend = MemoryNotify::default();
        let calls = backend.calls();
        let mut refused = Policy::new([NOTIFY]);
        refused.refuse(NOTIFY);
        let mut app = start(BODY, refused, backend);
        assert_eq!(app.go(), "err:denied:refused;err:denied:refused;denied:refused");
        assert!(calls.lock().unwrap().is_empty());

        let mut app = start(BODY, Policy::new(["store.kv"]), MemoryNotify::default());
        assert_eq!(app.go(), "err:denied:undeclared;err:denied:undeclared;denied:undeclared");
    }
}
