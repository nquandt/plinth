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
