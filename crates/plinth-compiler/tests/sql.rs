//! `store.sql` (core 1.13, `docs/SQL.md` §3, step 2 of §7) on the desktop
//! runner: `exec`, `query<T>`, `batch` and `migrate` from `plinth:store`,
//! rows decoded into `T` (a `boolean` reads 0 and 1), errors that reject the
//! promise, a denied call, the PL2026 warning, and FTS5. The suite runs on
//! wasmtime, and on wasmi with `PLINTH_ENGINE=wasmi`.

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

/// One app: `decls` at the top level, and `body` is the body of `async
/// function go()`, run by the button. `go` sets the text to `done:<r>`, or
/// to `done:err:<message>` when the body throws.
fn app_src(decls: &str, body: &str) -> String {
    format!(
        r#"import {{ app, Screen, Text, Button, signal }} from "plinth:ui";
import {{ sql }} from "plinth:store";
import {{ JSON }} from "plinth:core";
const result = signal("waiting");
{decls}
async function run(): Promise<string> {{
{body}
}}
async function go(): Promise<void> {{
  try {{
    const r = await run();
    result.set("done:" + r);
  }} catch (e) {{
    result.set("done:err:" + e.message);
  }}
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

const NOTES: &str = r#"
interface Note { id: number; title: string; done: boolean }
const SCHEMA = [
  "CREATE TABLE notes (id INTEGER PRIMARY KEY, title TEXT NOT NULL UNIQUE, done INTEGER NOT NULL DEFAULT 0)",
  "CREATE INDEX notes_done ON notes (done)",
];
const INSERT = "INSERT INTO notes (title, done) VALUES (?, ?)";
"#;

#[test]
fn exec_and_query_round_trip() {
    let body = r#"
  const v = await sql.migrate(SCHEMA);
  const a = await sql.exec(INSERT, ["Buy milk", false]);
  const b = await sql.exec(INSERT, ["Wörld 🌍", true]);
  const c = await sql.exec("UPDATE notes SET title = title || '!' WHERE id > ?", [0]);
  const rows = await sql.query<Note>("SELECT id, title, done FROM notes ORDER BY id");
  const open = await sql.query<Note>("SELECT id, title, done FROM notes WHERE done = ? ORDER BY id", [false]);
  return v + "|" + a.changes + "," + a.lastInsertId + "|" + b.lastInsertId + "|" + c.changes + "|" + JSON.stringify(rows) + "|" + open.length;
"#;
    assert_eq!(
        run_once("roundtrip", NOTES, body),
        r#"2|1,1|2|2|[{"id":1,"title":"Buy milk!","done":false},{"id":2,"title":"Wörld 🌍!","done":true}]|1"#
    );
}

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
fn query_reads_a_boolean_from_0_and_1_and_nullable_columns() {
    let decls = r#"
interface Row { flag: boolean; maybe: boolean | null; n: number | null; s: string | null }
"#;
    let body = r#"
  await sql.migrate(["CREATE TABLE t (id INTEGER PRIMARY KEY, flag INTEGER, maybe INTEGER, n REAL, s TEXT)"]);
  await sql.batch([
    { sql: "INSERT INTO t (flag, maybe, n, s) VALUES (?, ?, ?, ?)", params: [true, null, 1.5, "x"] },
    { sql: "INSERT INTO t (flag, maybe, n, s) VALUES (?, ?, ?, ?)", params: [0, 1, null, null] },
  ]);
  const rows = await sql.query<Row>("SELECT flag, maybe, n, s FROM t ORDER BY id");
  return JSON.stringify(rows);
"#;
    assert_eq!(
        run_once("bools", decls, body),
        r#"[{"flag":true,"maybe":null,"n":1.5,"s":"x"},{"flag":false,"maybe":true,"n":null,"s":null}]"#
    );
}

#[test]
fn a_row_that_does_not_match_the_type_rejects() {
    let decls = r#"
interface Wrong { id: string }
interface Missing { id: number; nope: string }
interface Flag { flag: boolean }
"#;
    let body = r#"
  await sql.migrate(["CREATE TABLE t (id INTEGER PRIMARY KEY, flag INTEGER)"]);
  await sql.exec("INSERT INTO t (flag) VALUES (2)");
  let r = "";
  try { await sql.query<Wrong>("SELECT id FROM t"); r = r + "ok;"; } catch (e) { r = r + e.message + ";"; }
  try { await sql.query<Missing>("SELECT id FROM t"); r = r + "ok;"; } catch (e) { r = r + e.message + ";"; }
  try { await sql.query<Flag>("SELECT flag FROM t"); r = r + "ok;"; } catch (e) { r = r + e.message + ";"; }
  const none = await sql.query<Wrong>("SELECT id FROM t WHERE id > 100");
  return r + none.length;
"#;
    assert_eq!(
        run_once("wrongtype", decls, body),
        "sql: a row does not match the type `Wrong[]`;sql: a row does not match the type `Missing[]`;sql: a row does not match the type `Flag[]`;0"
    );
}

#[test]
fn sql_errors_reject_and_the_app_keeps_running() {
    let body = r#"
  await sql.migrate(SCHEMA);
  let r = "";
  try { await sql.exec("SELEC 1"); r = r + "ok;"; } catch (e) { r = r + e.message.slice(0, 5) + ";"; }
  try { await sql.exec(INSERT, ["one"]); r = r + "ok;"; } catch (e) { r = r + e.message + ";"; }
  try { await sql.exec("ATTACH DATABASE 'x.db' AS x"); r = r + "ok;"; } catch (e) { r = r + e.message + ";"; }
  try { await sql.exec("BEGIN"); r = r + "ok;"; } catch (e) { r = r + e.message + ";"; }
  await sql.exec(INSERT, ["still works", false]);
  const rows = await sql.query<Note>("SELECT * FROM notes");
  return r + rows.length;
"#;
    assert_eq!(
        run_once("errors", NOTES, body),
        "sql: ;sql: the statement has 2 parameters, but the call gives 1;sql: not authorized;sql: not authorized;1"
    );
}

#[test]
fn a_failed_batch_step_rolls_back_all_steps() {
    let body = r#"
  await sql.migrate(SCHEMA);
  await sql.exec(INSERT, ["a", false]);
  let r = "";
  try {
    await sql.batch([
      { sql: INSERT, params: ["b", false] },
      { sql: "UPDATE notes SET done = 1" },
      { sql: INSERT, params: ["a", true] },
    ]);
    r = "ok";
  } catch (e) {
    r = e.message.startsWith("sql: ") ? "rejected" : e.message;
  }
  const rows = await sql.query<Note>("SELECT * FROM notes");
  // A batch from an array value.
  const steps: { sql: string; params: string[] }[] = [];
  for (const t of ["c", "d"]) {
    steps.push({ sql: "INSERT INTO notes (title) VALUES (?)", params: [t] });
  }
  await sql.batch(steps);
  const after = await sql.query<Note>("SELECT * FROM notes ORDER BY id");
  return r + "|" + JSON.stringify(rows) + "|" + after.map((n) => n.title).join(",");
"#;
    assert_eq!(
        run_once("batch", NOTES, body),
        r#"rejected|[{"id":1,"title":"a","done":false}]|a,c,d"#
    );
}

#[test]
fn migrate_runs_only_the_new_steps() {
    let body = r#"
  const steps = ["CREATE TABLE a (x INTEGER)", "INSERT INTO a (x) VALUES (1)"];
  const v1 = await sql.migrate(steps);
  const v2 = await sql.migrate(steps);
  const v3 = await sql.migrate(["CREATE TABLE a (x INTEGER)", "INSERT INTO a (x) VALUES (1)", "INSERT INTO a (x) VALUES (2)"]);
  const rows = await sql.query<{ x: number }>("SELECT x FROM a ORDER BY x");
  let r = "";
  try { await sql.migrate(["CREATE TABLE a (x INTEGER)"]); r = "ok"; } catch (e) { r = e.message; }
  return v1 + "," + v2 + "," + v3 + "|" + JSON.stringify(rows) + "|" + r;
"#;
    assert_eq!(
        run_once("migrate", "", body),
        r#"2,2,3|[{"x":1},{"x":2}]|sql: the database is at version 3, but there are only 1 migration steps (steps are append-only)"#
    );
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

#[test]
fn full_text_search_with_fts5() {
    let decls = r#"
interface Hit { title: string; snippet: string }
const SEARCH = "SELECT title, snippet(docs, 1, '[', ']', '…', 4) AS snippet FROM docs WHERE docs MATCH ? ORDER BY rank";
"#;
    let body = r#"
  await sql.migrate(["CREATE VIRTUAL TABLE docs USING fts5(title, body)"]);
  await sql.batch([
    { sql: "INSERT INTO docs (title, body) VALUES (?, ?)", params: ["Shopping", "buy milk and bread"] },
    { sql: "INSERT INTO docs (title, body) VALUES (?, ?)", params: ["Garden", "water the plants, then buy seeds"] },
    { sql: "INSERT INTO docs (title, body) VALUES (?, ?)", params: ["Work", "write the report"] },
  ]);
  const hits = await sql.query<Hit>(SEARCH, ["buy"]);
  const none = await sql.query<Hit>(SEARCH, ["nothing"]);
  return hits.map((h) => h.title).sort().join(",") + "|" + hits.length + "|" + none.length + "|" + hits.filter((h) => h.title === "Shopping")[0].snippet;
"#;
    assert_eq!(
        run_once("fts5", decls, body),
        "Garden,Shopping|2|0|[buy] milk and bread"
    );
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
