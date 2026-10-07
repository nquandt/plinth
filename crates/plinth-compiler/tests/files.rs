//! `plinth:files` on the private space (core 1.11, `docs/STORAGE.md` §2,
//! §3, §6 item 5) on the desktop runner: the promise and callback forms,
//! the walls between apps (another app id, another publisher, an unsigned
//! package), path tricks, and denied calls that never trap.

use plinth_compiler::driver::MemFs;
use plinth_protocol::{ControlKind, Event, NodeId, Value, Writer, event};
use plinth_runner_wasmtime::files::{DEFAULT_QUOTA, FILES_PRIVATE, Files, Owner, private_space_dir};
use plinth_runner_wasmtime::policy::Policy;
use plinth_runner_wasmtime::{Guest, Limits, Runner, kv::Kv};
use plinth_ui::tree::{Node, Tree};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn compile(src: &str, caps: &[&str]) -> plinth_compiler::Artifact {
    let fs = MemFs::default().with("app/main.tsx", src);
    let caps: Vec<String> = caps.iter().map(|s| s.to_string()).collect();
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &caps).expect("compile");
    artifact.unwrap_or_else(|| {
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        panic!("compile errors:\n{}", diags.join("\n"))
    })
}

/// One app: `body` is the body of `async function go()`, run by the button.
fn app_src(body: &str) -> String {
    format!(
        r#"import {{ app, Screen, Text, Button, signal }} from "plinth:ui";
import {{ read, write, list, stat, remove }} from "plinth:files";
import {{ JSON }} from "plinth:core";
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

fn start(src: &str, policy: Policy, files: Files, gc_stress: bool) -> App {
    let artifact = compile(src, &[FILES_PRIVATE]);
    let runner = Runner::new().unwrap();
    let mut guest = runner
        .load_with_policy(&artifact.component, Limits::default(), policy, Kv::in_memory(), Box::new(plinth_runner_wasmtime::MemoryClipboard::default()))
        .unwrap();
    guest.set_files(files);
    let args = if gc_stress { plinth_protocol::init_arg::one(plinth_protocol::init_arg::GC_STRESS, &[]) } else { Vec::new() };
    let mut tree = Tree::new();
    for commit in guest.init(&args).unwrap() {
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
    /// Presses the button, then delivers completions until the text starts
    /// with `done:`, and returns the text after it.
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

fn data_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("plinth-files-it-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn space(data: &Path, owner: &Owner, id: &str) -> Files {
    Files::open(private_space_dir(data, owner, id), DEFAULT_QUOTA)
}

fn allowed() -> Policy {
    Policy::new([FILES_PRIVATE])
}

/// `try { <expr> } catch (e) { r = "err:" + e.message }`, appended to `r`.
fn attempt(expr: &str) -> String {
    format!(r#"try {{ {expr}; r = r + "ok;"; }} catch (e) {{ r = r + "err:" + e.message + ";"; }}"#)
}

#[test]
fn promise_form_round_trip() {
    let data = data_dir("promise");
    let owner = Owner::Publisher("ed25519:alice".into());
    let body = r#"
  await write("notes/a.md", "hello");
  await write("notes/b.md", "wörld");
  await write("top.txt", "1");
  const t = await read("notes/b.md");
  const root = await list("");
  const notes = await list("notes");
  const s = await stat("notes/a.md");
  const none = await stat("nope");
  await remove("notes/a.md");
  const after = await list("notes");
  result.set("done:" + t + "|" + JSON.stringify(root) + "|" + JSON.stringify(notes) + "|" + (s === null ? "null" : s.kind + s.size) + "|" + (none === null ? "null" : "x") + "|" + after.length);
"#;
    let mut app = start(&app_src(body), allowed(), space(&data, &owner, "com.example.a"), false);
    assert_eq!(
        app.go(),
        r#"wörld|[{"name":"notes","kind":"dir","size":0},{"name":"top.txt","kind":"file","size":1}]|[{"name":"a.md","kind":"file","size":5},{"name":"b.md","kind":"file","size":6}]|file5|null|1"#
    );
    // The data is on disk, in the folder of this identity.
    let dir = private_space_dir(&data, &owner, "com.example.a");
    assert_eq!(std::fs::read_to_string(dir.join("notes").join("b.md")).unwrap(), "wörld");
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn callback_form_round_trip() {
    let data = data_dir("callback");
    let body = r#"
  write("x.md", "cb", (e) => {
    read("x.md", (e2, text) => {
      list("", (e3, entries) => {
        read("missing.md", (e4, t4) => {
          result.set("done:" + (e ?? "null") + "|" + text + "|" + entries.length + "|" + (e4 ?? "null") + "|" + t4.length);
        });
      });
    });
  });
"#;
    let mut app = start(&app_src(body), allowed(), space(&data, &Owner::Dev, "com.example.cb"), false);
    assert_eq!(app.go(), "null|cb|1|not-found|0");
    let _ = std::fs::remove_dir_all(&data);
}

/// STORAGE.md §6 item 5 (private part): app A writes; app B (another id),
/// the same id from another publisher, and the same id unsigned see none of
/// A's files, also with path tricks.
#[test]
fn apps_cannot_reach_each_others_files() {
    let data = data_dir("isolation");
    let alice = Owner::Publisher("ed25519:alice".into());
    let mallory = Owner::Publisher("ed25519:mallory".into());
    let unsigned = Owner::of_package(None, b"a package with the same id");

    let mut a = start(&app_src(r#"await write("secret.md", "A's secret"); result.set("done:written");"#), allowed(), space(&data, &alice, "com.example.a"), false);
    assert_eq!(a.go(), "written");
    let a_dir = private_space_dir(&data, &alice, "com.example.a");
    assert!(a_dir.join("secret.md").is_file());

    // The folder name of A, as a tricky app might guess it.
    let a_name = a_dir.file_name().unwrap().to_string_lossy().into_owned();
    let a_owner = a_dir.parent().unwrap().file_name().unwrap().to_string_lossy().into_owned();
    let tricks = [
        "secret.md".to_owned(),
        format!("../{a_name}/secret.md"),
        format!("../../{a_owner}/{a_name}/secret.md"),
        format!("..\\\\{a_name}\\\\secret.md"),
        a_dir.join("secret.md").to_string_lossy().replace('\\', "/"),
        format!("{}", a_dir.join("secret.md").to_string_lossy().replace('\\', "\\\\")),
        "./secret.md".to_owned(),
        "secret.md\\u0000".to_owned(),
        "C:secret.md".to_owned(),
        "/secret.md".to_owned(),
    ];
    let reads: String = tricks.iter().map(|p| attempt(&format!(r#"const t{0} = await read("{p}"); r = r + t{0}"#, p.len()))).collect::<Vec<_>>().join("\n");
    let body = format!(
        r#"let r = "";
  const entries = await list("");
  r = r + "list:" + entries.length + ";";
{reads}
  {}
  result.set("done:" + r);"#,
        attempt(r#"await write("../escape.txt", "x")"#)
    );

    for (name, owner, id) in [("another id", &alice, "com.example.b"), ("another publisher", &mallory, "com.example.a"), ("unsigned", &unsigned, "com.example.a")] {
        let mut b = start(&app_src(&body), allowed(), space(&data, owner, id), false);
        let out = b.go();
        assert!(!out.contains("A's secret"), "{name}: {out}");
        assert!(out.starts_with("list:0;err:not-found;"), "{name}: {out}");
        let parts: Vec<&str> = out.trim_end_matches(';').split(';').collect();
        assert_eq!(parts.len(), 2 + tricks.len(), "{name}: {out}");
        for (p, trick) in parts[2..parts.len() - 1].iter().zip(&tricks[1..]) {
            assert!(p.starts_with("err:invalid-path"), "{name}: {trick:?} gave {p}");
        }
        assert!(parts.last().unwrap().starts_with("err:invalid-path"), "{name}: {out}");
    }
    assert!(!private_space_dir(&data, &alice, "com.example.b").parent().unwrap().join("escape.txt").exists());
    assert!(!data.join("spaces").join("private").join("escape.txt").exists());
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn a_denied_call_rejects_and_the_app_keeps_running() {
    let data = data_dir("denied");
    let body = format!(r#"let r = ""; {} {} result.set("done:" + r);"#, attempt(r#"await write("a.md", "x")"#), attempt(r#"const l = await list(""); r = r + l.length"#));
    // Declared in the package, but the host policy does not have it.
    let mut app = start(&app_src(&body), Policy::new(Vec::<String>::new()), space(&data, &Owner::Dev, "com.example.d"), false);
    assert_eq!(app.go(), "err:denied:undeclared;err:denied:undeclared;");
    // The same app again: it did not trap.
    assert_eq!(app.go(), "err:denied:undeclared;err:denied:undeclared;");

    let mut refused = allowed();
    refused.refuse(FILES_PRIVATE);
    let mut app = start(&app_src(&body), refused, space(&data, &Owner::Dev, "com.example.d"), false);
    assert_eq!(app.go(), "err:denied:refused;err:denied:refused;");

    let mut app = start(&app_src(&body), allowed(), Files::unavailable(), false);
    assert_eq!(app.go(), "err:denied:unsupported;err:denied:unsupported;");
    assert!(!data.exists(), "a denied call wrote nothing");
}

#[test]
fn the_callbacks_survive_a_gc_between_the_call_and_its_answer() {
    let data = data_dir("gc");
    let body = r#"
  const parts: string[] = [];
  for (let i = 0; i < 5; i++) {
    await write("n" + i + ".txt", "v" + i);
    parts.push(await read("n" + i + ".txt"));
  }
  const l = await list("");
  result.set("done:" + parts.join(",") + ":" + l.length);
"#;
    let mut app = start(&app_src(body), allowed(), space(&data, &Owner::Dev, "com.example.gc"), true);
    assert_eq!(app.go(), "v0,v1,v2,v3,v4:5");
    let _ = std::fs::remove_dir_all(&data);
}
