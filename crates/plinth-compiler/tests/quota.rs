//! The per-app data quota (`docs/STORAGE.md` S13) on the desktop runner:
//! the same number limits the private files, `store.kv` and the `store.sql`
//! database, and a write over it gives an error that the app can read. The
//! host gives the number (`plinth_hub::data_quota`); here the test gives a
//! small one.

use plinth_compiler::driver::MemFs;
use plinth_protocol::{ControlKind, Event, NodeId, Value, Writer, event};
use plinth_runner_wasmtime::files::{FILES_PRIVATE, Files, Owner, private_space_dir};
use plinth_runner_wasmtime::policy::Policy;
use plinth_runner_wasmtime::sql::STORE_SQL;
use plinth_runner_wasmtime::{Guest, Limits, Runner, kv::Kv};
use plinth_ui::tree::{Node, Tree};
use std::time::{Duration, Instant};

const QUOTA: u64 = 64 << 10;

const APP: &str = r#"
import { app, Screen, Text, Button, signal } from "plinth:ui";
import { kv, sql } from "plinth:store";
import { write } from "plinth:files";
const result = signal("waiting");
const BIG = "x".repeat(40000);
async function run(): Promise<string> {
  let r = "";
  // kv: 40 KB fits, a second 40 KB does not; the error is readable.
  kv.set("a", BIG);
  r = r + "kv1=" + (kv.lastError() ?? "ok") + ";";
  kv.set("b", BIG);
  r = r + "kv2=" + (kv.lastError() ?? "ok") + ";";
  r = r + "b=" + (kv.get("b") === null ? "null" : "set") + ";";
  // A smaller value that frees space works, then the second key fits.
  kv.set("a", "small");
  r = r + "kv3=" + (kv.lastError() ?? "ok") + ";";
  kv.set("b", BIG);
  r = r + "kv4=" + (kv.lastError() ?? "ok") + ";";
  // Files: the same. The quota is for all stores together, so the kv
  // store gets small first (the combined quota has its own test below).
  kv.set("b", "small");
  await write("one.txt", BIG.slice(0, 30000));
  r = r + "f1=ok;";
  try { await write("two.txt", BIG); r = r + "f2=ok;"; } catch (e) { r = r + "f2=" + e.message + ";"; }
  // sql: rows of 4 KB until the database is full.
  await sql.migrate(["CREATE TABLE t (v TEXT)"]);
  let n = 0;
  try {
    for (let i = 0; i < 40; i = i + 1) {
      await sql.exec("INSERT INTO t VALUES (?)", [BIG.slice(0, 4000)]);
      n = n + 1;
    }
    r = r + "sql=never full;";
  } catch (e) {
    r = r + "sql=" + e.message + ";";
  }
  r = r + (n > 3 && n < 40 ? "rows ok" : "rows=" + n);
  return r;
}
async function go(): Promise<void> {
  try {
    result.set("done:" + (await run()));
  } catch (e) {
    result.set("done:err:" + e.message);
  }
}
function Home() {
  return <Screen title="Home">
    <Text>{result()}</Text>
    <Button label="go" onPress={() => { go(); }} />
  </Screen>;
}
export default app({ screens: { home: { title: "Home", component: Home } } });
"#;

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

/// Runs `app` with the 64 KiB quota and gives the text that it ends with.
fn run_app(app: &str, data_name: &str) -> String {
    let fs = MemFs::default().with("app/main.tsx", app);
    let caps: Vec<String> = ["store.kv", STORE_SQL, FILES_PRIVATE].iter().map(|s| s.to_string()).collect();
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &caps).expect("compile");
    let artifact = artifact.unwrap_or_else(|| {
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        panic!("compile errors:\n{}", diags.join("\n"))
    });
    let data = std::env::temp_dir().join(format!("plinth-quota-it-{data_name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);

    let runner = Runner::new().unwrap();
    let policy = Policy::new(caps.iter().map(String::as_str));
    let mut guest: Guest = runner
        .load_with_policy(
            &artifact.component,
            Limits::default(),
            policy,
            Kv::in_memory().with_quota(QUOTA),
            Box::new(plinth_runner_wasmtime::MemoryClipboard::default()),
        )
        .unwrap();
    guest.set_files(Files::open(private_space_dir(&data, &Owner::Dev, "com.example.quota"), QUOTA));
    let mut tree = Tree::new();
    for commit in guest.init(&[]).unwrap() {
        assert!(tree.apply(&commit).unwrap().is_empty());
    }

    let button = find(&tree, ControlKind::Button, |_| true);
    let handler = tree.get(button).and_then(|n| n.handler(event::PRESS)).unwrap();
    let mut w = Writer::new();
    w.event(&Event::Ui { handler, event: event::PRESS, value: Value::Null });
    for commit in guest.on_event(w.as_bytes()).unwrap() {
        assert!(tree.apply(&commit).unwrap().is_empty());
    }
    let start = Instant::now();
    let result = loop {
        if let Some(rest) = text(&tree).strip_prefix("done:") {
            break rest.to_owned();
        }
        for (id, r) in guest.poll_net_results() {
            for commit in guest.answer_dialog(id, r).unwrap() {
                assert!(tree.apply(&commit).unwrap().is_empty());
            }
        }
        assert!(start.elapsed() < Duration::from_secs(30), "no result; text is {:?}", text(&tree));
        std::thread::sleep(Duration::from_millis(2));
    };
    drop(guest);
    let _ = std::fs::remove_dir_all(&data);
    result
}

#[test]
fn one_quota_limits_files_kv_and_sql_and_the_app_can_read_the_error() {
    assert_eq!(
        run_app(APP, "each"),
        "kv1=ok;kv2=denied:refused;b=null;kv3=ok;kv4=ok;f1=ok;f2=quota;sql=sql: the database is full;rows ok"
    );
}

const COMBINED: &str = r#"
import { app, Screen, Text, Button, signal } from "plinth:ui";
import { kv, sql } from "plinth:store";
import { write } from "plinth:files";
const result = signal("waiting");
async function run(): Promise<string> {
  let r = "";
  // Each store alone stays under the quota (64 KiB); together they pass it.
  kv.set("a", "x".repeat(40000));
  r = r + "kv1=" + (kv.lastError() ?? "ok") + ";";
  await write("one.txt", "y".repeat(15000));
  r = r + "f1=ok;";
  try { await write("two.txt", "y".repeat(15000)); r = r + "f2=ok;"; } catch (e) { r = r + "f2=" + e.message + ";"; }
  kv.set("c", "z".repeat(12000));
  r = r + "kv2=" + (kv.lastError() ?? "ok") + ";";
  // A smaller value frees room and works.
  kv.set("a", "small");
  r = r + "kv3=" + (kv.lastError() ?? "ok") + ";";
  kv.set("a", "x".repeat(40000));
  r = r + "kv4=" + (kv.lastError() ?? "ok") + ";";
  await sql.migrate(["CREATE TABLE t (v TEXT)"]);
  let n = 0;
  try {
    for (let i = 0; i < 40; i = i + 1) {
      await sql.exec("INSERT INTO t VALUES (?)", ["q".repeat(4000)]);
      n = n + 1;
    }
    r = r + "sql=never full;";
  } catch (e) {
    r = r + "sql=" + e.message + ";";
  }
  r = r + (n <= 2 ? "rows ok" : "rows=" + n);
  return r;
}
async function go(): Promise<void> {
  try {
    result.set("done:" + (await run()));
  } catch (e) {
    result.set("done:err:" + e.message);
  }
}
function Home() {
  return <Screen title="Home">
    <Text>{result()}</Text>
    <Button label="go" onPress={() => { go(); }} />
  </Screen>;
}
export default app({ screens: { home: { title: "Home", component: Home } } });
"#;

/// The quota is for all stores of one app together (`docs/STORAGE.md` §7):
/// kv 40000 B, files 15000 B and a database are each under 64 KiB alone.
#[test]
fn the_quota_is_for_all_stores_together() {
    assert_eq!(
        run_app(COMBINED, "combined"),
        "kv1=ok;f1=ok;f2=quota;kv2=denied:refused;kv3=ok;kv4=ok;sql=sql: the database is full;rows ok"
    );
}
