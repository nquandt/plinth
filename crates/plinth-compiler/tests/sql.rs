//! `store.sql` (core 1.13, `docs/SQL.md` §3, step 2 of §7) on the desktop
//! runner: `exec`, `query<T>`, `batch` and `migrate` from `plinth:store`,
//! rows decoded into `T` (a `boolean` reads 0 and 1), errors that reject the
//! promise, a denied call, the PL2026 warning, and FTS5. The suite runs on
//! wasmtime, and on wasmi with `PLINTH_ENGINE=wasmi`.
//!
//! The app cases are in `sql_cases.txt`, which the web test
//! (`web/test/run-sql.mjs`) also runs: both hosts must give the same text
//! (step 3 of `docs/SQL.md` §7).

use plinth_compiler::driver::MemFs;
use plinth_protocol::{ControlKind, Event, NodeId, Value, Writer, event};
use plinth_runner_wasmtime::files::{DEFAULT_QUOTA, Files, Owner, private_space_dir};
use plinth_runner_wasmtime::policy::Policy;
use plinth_runner_wasmtime::sql::{STORE_SQL, database_dir};
use plinth_runner_wasmtime::{Guest, Limits, Runner, kv::Kv};
use plinth_ui::tree::{Node, Tree};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn front(src: &str) -> plinth_compiler::driver::Frontend {
    let fs = MemFs::default().with("app/main.tsx", src);
    plinth_compiler::driver::frontend_with_capabilities(&fs, &[STORE_SQL.to_owned()])
}

fn compile(src: &str) -> plinth_compiler::Artifact {
    let fs = MemFs::default().with("app/main.tsx", src);
    let (front, artifact) =
        plinth_compiler::compile_with_capabilities(&fs, &[STORE_SQL.to_owned()]).expect("compile");
    artifact.unwrap_or_else(|| {
        let diags: Vec<String> = front
            .diags
            .iter()
            .map(|d| front.sources.render(d))
            .collect();
        panic!("compile errors:\n{}", diags.join("\n"))
    })
}

/// The shared cases (`tests/sql_cases.txt`): the web test
/// (`web/test/run-sql.mjs`) reads the same file and must give the same text.
struct Cases {
    template: String,
    cases: Vec<Case>,
}

struct Case {
    name: String,
    decls: String,
    body: String,
    expect: String,
}

fn cases() -> Cases {
    let text = include_str!("sql_cases.txt").replace("\r\n", "\n");
    let mut template: Vec<&str> = Vec::new();
    let mut cases: Vec<(String, [Vec<&str>; 3])> = Vec::new();
    // 0 template, 1 a case with no field yet, 2.. the fields of a case.
    let mut at: Option<usize> = None;
    for line in text.split('\n') {
        if line == "=== template" {
            at = Some(0);
            continue;
        }
        if let Some(name) = line.strip_prefix("=== case ") {
            cases.push((name.to_owned(), Default::default()));
            at = Some(1);
            continue;
        }
        if at.is_some_and(|a| a >= 1) {
            let field = match line {
                "--- decls" => Some(2),
                "--- body" => Some(3),
                "--- expect" => Some(4),
                _ => None,
            };
            if let Some(f) = field {
                at = Some(f);
                continue;
            }
        }
        match at {
            Some(0) => template.push(line),
            Some(f @ 2..=4) => cases.last_mut().unwrap().1[f - 2].push(line),
            _ => assert!(
                line.trim().is_empty() || line.starts_with('#'),
                "sql_cases.txt: a line outside a section: {line}"
            ),
        }
    }
    let join = |mut lines: Vec<&str>| {
        while lines.last() == Some(&"") {
            lines.pop();
        }
        lines.join("\n")
    };
    Cases {
        template: join(template),
        cases: cases
            .into_iter()
            .map(|(name, [decls, body, expect])| Case {
                name,
                decls: join(decls),
                body: join(body),
                expect: join(expect),
            })
            .collect(),
    }
}

/// One app: `decls` at the top level, and `body` is the body of `async
/// function run()`, run by the button. The button sets the text to
/// `done:<r>`, or to `done:err:<message>` when the body throws.
fn app_src(decls: &str, body: &str) -> String {
    cases()
        .template
        .replacen("{decls}", decls, 1)
        .replacen("{body}", body, 1)
}

struct App {
    guest: Guest,
    tree: Tree,
}

fn start(src: &str, policy: Policy, files: Files) -> App {
    let artifact = compile(src);
    let runner = Runner::new().unwrap();
    let mut guest = runner
        .load_with_policy(
            &artifact.component,
            Limits::default(),
            policy,
            Kv::in_memory(),
            Box::new(plinth_runner_wasmtime::MemoryClipboard::default()),
        )
        .unwrap();
    guest.set_files(files);
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
    /// Presses the button, then delivers completions until the text starts
    /// with `done:`, and returns the text after it.
    fn go(&mut self) -> String {
        let button = find(&self.tree, ControlKind::Button, |_| true);
        let handler = self
            .tree
            .get(button)
            .and_then(|n| n.handler(event::PRESS))
            .unwrap();
        let mut w = Writer::new();
        w.event(&Event::Ui {
            handler,
            event: event::PRESS,
            value: Value::Null,
        });
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
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "no result; text is {:?}",
                text(&self.tree)
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

/// A fresh data folder and the private space of `id` in it.
fn space(name: &str, id: &str) -> (PathBuf, Files) {
    let data = std::env::temp_dir().join(format!("plinth-sql-it-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    let files = Files::open(private_space_dir(&data, &Owner::Dev, id), DEFAULT_QUOTA);
    (data, files)
}

fn allowed() -> Policy {
    Policy::new([STORE_SQL])
}

/// Runs `body` once in a new app with a new database, and gives the result.
fn run_once(name: &str, decls: &str, body: &str) -> String {
    let (data, files) = space(name, "com.example.sql");
    let mut app = start(&app_src(decls, body), allowed(), files);
    let r = app.go();
    drop(app);
    let _ = std::fs::remove_dir_all(&data);
    r
}

#[test]
fn the_shared_cases_give_the_expected_text() {
    let all = cases();
    assert!(all.cases.len() >= 8, "sql_cases.txt has the cases");
    for case in &all.cases {
        assert_eq!(
            run_once(&case.name, &case.decls, &case.body),
            case.expect,
            "case {}",
            case.name
        );
    }
}

const NOTES: &str = r#"
interface Note { id: number; title: string; done: boolean }
const SCHEMA = [
  "CREATE TABLE notes (id INTEGER PRIMARY KEY, title TEXT NOT NULL UNIQUE, done INTEGER NOT NULL DEFAULT 0)",
  "CREATE INDEX notes_done ON notes (done)",
];
const INSERT = "INSERT INTO notes (title, done) VALUES (?, ?)";
"#;

#[test]
fn the_database_is_on_disk_next_to_the_private_space() {
    let (data, files) = space("disk", "com.example.disk");
    let db_dir = database_dir(&private_space_dir(&data, &Owner::Dev, "com.example.disk"));
    let body = r#"
  await sql.migrate(SCHEMA);
  await sql.exec(INSERT, ["kept", 0]);
  const rows = await sql.query<Note>("SELECT * FROM notes");
  return rows[0].title;
"#;
    let mut app = start(&app_src(NOTES, body), allowed(), files);
    assert_eq!(app.go(), "kept");
    drop(app);
    assert!(db_dir.join("store.sqlite").is_file());

    // A second run of the app sees the same data, and the migration does not
    // run again.
    let files = Files::open(
        private_space_dir(&data, &Owner::Dev, "com.example.disk"),
        DEFAULT_QUOTA,
    );
    let body = r#"
  const v = await sql.migrate(SCHEMA);
  const rows = await sql.query<Note>("SELECT * FROM notes");
  return v + ":" + rows.length + ":" + rows[0].title;
"#;
    let mut app = start(&app_src(NOTES, body), allowed(), files);
    assert_eq!(app.go(), "2:1:kept");
    drop(app);
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn a_call_without_the_capability_is_denied_and_does_not_trap() {
    let (data, files) = space("denied", "com.example.denied");
    let body = r#"
  let r = "";
  try { await sql.migrate(["CREATE TABLE a (x INTEGER)"]); r = "ok;"; } catch (e) { r = e.message + ";"; }
  try { await sql.query<{ x: number }>("SELECT 1 AS x"); r = r + "ok"; } catch (e) { r = r + e.message; }
  return r;
"#;
    let mut app = start(&app_src("", body), Policy::new(Vec::<&str>::new()), files);
    assert_eq!(app.go(), "denied:undeclared;denied:undeclared");
    drop(app);
    // The host made no database.
    assert!(!database_dir(&private_space_dir(&data, &Owner::Dev, "com.example.denied")).exists());
    let _ = std::fs::remove_dir_all(&data);

    // A host with no private space for the app.
    let body = r#"
  try { await sql.exec("SELECT 1"); return "ok"; } catch (e) { return e.message; }
"#;
    let mut app = start(&app_src("", body), allowed(), Files::unavailable());
    assert_eq!(app.go(), "denied:unsupported");
}

fn codes(src: &str) -> Vec<&'static str> {
    front(src).diags.iter().map(|d| d.code).collect()
}

#[test]
fn a_statement_that_is_not_a_literal_or_a_const_gives_a_warning() {
    // A literal, a template with no values, and `const` statements: no warning.
    let good = app_src(
        NOTES,
        r#"
  await sql.migrate(SCHEMA);
  await sql.migrate(["CREATE TABLE a (x INTEGER)"]);
  await sql.exec(INSERT, ["a", false]);
  await sql.exec(`DELETE FROM notes`);
  const q = "SELECT * FROM notes WHERE id = ?";
  const rows = await sql.query<Note>(q, [1]);
  await sql.batch([{ sql: INSERT, params: ["b", true] }]);
  return "" + rows.length;
"#,
    );
    assert_eq!(codes(&good), Vec::<&str>::new());

    // A statement built from strings, a `let`, a template with a value, and
    // a parameter of a function.
    let bad = app_src(
        NOTES,
        r#"
  const table = "notes";
  let q = "SELECT * FROM notes";
  await sql.exec("DELETE FROM " + table);
  await sql.query<Note>(q);
  await sql.query<Note>(`SELECT * FROM ${table}`);
  await sql.batch([{ sql: "DELETE FROM " + table }]);
  await sql.migrate(["CREATE TABLE " + table + "2 (x INTEGER)"]);
  return "";
"#,
    );
    assert_eq!(
        codes(&bad),
        ["PL2026", "PL2026", "PL2026", "PL2026", "PL2026"]
    );
    let f = front(&bad);
    let w = f.diags.iter().find(|d| d.code == "PL2026").unwrap();
    assert_eq!(w.severity, plinth_compiler::diag::Severity::Warning);
    // A warning does not stop the build.
    compile(&bad);
}

#[test]
fn wrong_calls_are_compile_errors() {
    let cases = [
        // No type argument for the rows.
        ("await sql.query(\"SELECT 1\");", "PL2015"),
        // Rows of a type that is not an object.
        ("await sql.query<number>(\"SELECT 1\");", "PL3001"),
        // A parameter that is not a SQL value.
        ("await sql.exec(\"SELECT ?\", [[1]]);", "PL3001"),
        // Parameters that are not an array.
        ("await sql.exec(\"SELECT ?\", 1);", "PL3001"),
        // No such call.
        ("await sql.run(\"SELECT 1\");", "PL3004"),
        // Too many arguments.
        ("await sql.exec(\"SELECT 1\", [], 3);", "PL3006"),
    ];
    for (body, code) in cases {
        let src = app_src("", &format!("{body}\n  return \"\";"));
        assert_eq!(codes(&src), [code], "{body}");
    }
    // The capability is required (PL1007).
    let src = app_src("", "await sql.exec(\"SELECT 1\");\n  return \"\";");
    let fs = MemFs::default().with("app/main.tsx", &src);
    let f = plinth_compiler::driver::frontend_with_capabilities(&fs, &[]);
    assert_eq!(
        f.diags.iter().map(|d| d.code).collect::<Vec<_>>(),
        ["PL1007"]
    );
}

#[test]
fn a_batch_of_1000_steps_from_an_array_value() {
    // The notes example moves 1,000 old notes in one batch (docs/SQL.md §7 step 4).
    let body = r#"
  await sql.migrate(["CREATE TABLE t (id INTEGER PRIMARY KEY, title TEXT NOT NULL, body TEXT NOT NULL)"]);
  const steps: { sql: string; params: string[] }[] = [];
  for (let i = 0; i < 1000; i += 1) {
    steps.push({ sql: "INSERT INTO t (title, body) VALUES (?, ?)", params: [`Note ${i}`, `topic${i % 50} and some more words for the body of note ${i}`] });
  }
  await sql.batch(steps);
  const rows = await sql.query<{ n: number }>("SELECT count(*) AS n FROM t");
  return "" + rows[0].n;
"#;
    assert_eq!(run_once("batch1000", "", body), "1000");
}

#[test]
fn json_stringify_of_a_large_array_or_map_does_not_run_out_of_memory() {
    // The collector does not run during a call. `JSON.stringify` of an array
    // or a map joins its parts one time: before, it added each part to a
    // longer string, and a text of about 100 KB went over the 64 MiB limit.
    let body = r#"
  const steps: { sql: string; params: string[] }[] = [];
  const m = new Map<string, string>();
  for (let i = 0; i < 2000; i += 1) {
    steps.push({ sql: "INSERT INTO t (title, body) VALUES (?, ?)", params: [`Note ${i}`, `topic${i % 50} and some more words for the body of note ${i}`] });
    m.set(`key ${i}`, `some value text for the entry number ${i}`);
  }
  const text = JSON.stringify(steps);
  const back = JSON.parse<{ sql: string; params: string[] }[]>(text);
  const mapText = JSON.stringify(m);
  if (back === null) return "null";
  return text.length + ":" + back.length + ":" + back[1999].params[0] + ":" + mapText.length + ":" + mapText.slice(0, 40);
"#;
    let r = run_once("stringify2000", "", body);
    let parts: Vec<&str> = r.split(':').collect();
    assert!(parts[0].parse::<usize>().unwrap() > 200_000, "{r}");
    assert_eq!(parts[1], "2000", "{r}");
    assert_eq!(parts[2], "Note 1999", "{r}");
    assert!(parts[3].parse::<usize>().unwrap() > 100_000, "{r}");
    assert_eq!(&r[r.find(":{").unwrap() + 1..], r#"{"key 0":"some value text for the entry "#, "{r}");
}
