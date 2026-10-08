//! `examples/notes` on the desktop runner with `store.sql` (docs/SQL.md §7
//! step 4, docs/VALIDATION.md V2): the notes are in a SQL table, the
//! search uses the FTS5 index (a prefix search on each word), the list is
//! sorted by the last change, the notes of an older build move from
//! `store.kv` into the table, and the data stays for the next run.

use plinth_compiler::driver::DiskFs;
use plinth_protocol::{ControlKind, Event, NodeId, Value, Writer, event, prop};
use plinth_runner_wasmtime::files::{DEFAULT_QUOTA, Files, Owner, private_space_dir};
use plinth_runner_wasmtime::kv::Kv;
use plinth_runner_wasmtime::policy::Policy;
use plinth_runner_wasmtime::{Guest, Limits, MemoryClipboard, Runner};
use plinth_ui::tree::{Node, Tree};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const ID: &str = "dev.plinth.examples.notes";
const FIRST: [&str; 3] = ["Welcome", "Shopping list", "Idea"];

fn artifact() -> &'static plinth_compiler::Artifact {
    static ART: OnceLock<plinth_compiler::Artifact> = OnceLock::new();
    ART.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/notes");
        let fs = DiskFs { root };
        let caps = ["store.sql".to_owned(), "store.kv".to_owned()];
        let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &caps).expect("compile");
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        assert!(diags.is_empty(), "notes has diagnostics:\n{}", diags.join("\n"));
        artifact.expect("notes compiles")
    })
}

/// A new, empty data folder for one test.
fn data_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("plinth-notes-it-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct Notes {
    guest: Guest,
    tree: Tree,
    _runner: Runner,
}

impl Notes {
    /// Starts the app on the data folder `data` (its database and its kv
    /// file), and waits until the list shows the notes.
    fn start(data: &Path) -> Self {
        let runner = Runner::new().unwrap();
        let kv = Kv::open(data, ID).unwrap();
        let policy = Policy::new(["store.sql", "store.kv"]);
        let mut guest = runner
            .load_with_policy(&artifact().component, Limits::default(), policy, kv, Box::new(MemoryClipboard::default()))
            .unwrap();
        guest.set_files(Files::open(private_space_dir(data, &Owner::Dev, ID), DEFAULT_QUOTA));
        let mut tree = Tree::new();
        let commits = guest.init(&[]).unwrap();
        for commit in commits {
            assert!(tree.apply(&commit).unwrap().is_empty());
        }
        let mut app = Self { guest, tree, _runner: runner };
        app.wait("the notes load", |a| !a.status().starts_with("Loading"));
        app
    }

    /// Delivers the answers of the database until `done` is true.
    fn wait(&mut self, what: &str, done: impl Fn(&Self) -> bool) -> Duration {
        let start = Instant::now();
        loop {
            if done(self) {
                let errors = self.guest.take_errors();
                assert!(errors.is_empty(), "uncaught errors: {errors:?}");
                return start.elapsed();
            }
            for (id, result) in self.guest.poll_net_results() {
                for commit in self.guest.answer_dialog(id, result).unwrap() {
                    assert!(self.tree.apply(&commit).unwrap().is_empty());
                }
            }
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "{what}: timed out; the list is {:?}, the status is {:?}",
                self.rows(),
                self.status()
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Waits until the list shows `rows`, and the count text is good.
    fn wait_rows(&mut self, what: &str, rows: &[&str]) -> Duration {
        // Each problem text of the app has a ':' ("Search failed: ...").
        let t = self.wait(what, |a| a.rows() == rows || a.status().contains(':'));
        assert_eq!(self.rows(), rows, "{what}: the status is {:?}", self.status());
        assert!(self.status().ends_with(" notes"), "{what}: the status is {:?}", self.status());
        t
    }

    fn fire(&mut self, node: NodeId, ev: u16, value: Value) {
        let handler = self.tree.get(node).and_then(|n| n.handler(ev)).expect("node has a handler");
        let mut w = Writer::new();
        w.event(&Event::Ui { handler, event: ev, value });
        for commit in self.guest.on_event(w.as_bytes()).unwrap() {
            assert!(self.tree.apply(&commit).unwrap().is_empty());
        }
    }

    fn find(&self, kind: ControlKind, pred: impl Fn(&Node) -> bool) -> Vec<NodeId> {
        let mut found = Vec::new();
        let mut stack: Vec<NodeId> = self.tree.screens().map(|(_, id)| id).collect();
        stack.reverse();
        while let Some(id) = stack.pop() {
            let node = self.tree.get(id).unwrap();
            if node.kind == Some(kind) && pred(node) {
                found.push(id);
            }
            stack.extend(node.children.iter().rev());
        }
        found
    }

    fn section(&self, title: &str) -> Option<NodeId> {
        self.find(ControlKind::Section, |n| n.str_prop(prop::TITLE) == Some(title)).first().copied()
    }

    /// The field labelled `label` in the section `section`.
    fn field(&self, section: &str, label: &str) -> NodeId {
        let s = self.section(section).unwrap_or_else(|| panic!("no section {section}"));
        *self
            .tree
            .get(s)
            .unwrap()
            .children
            .iter()
            .find(|c| self.tree.get(**c).unwrap().str_prop(prop::LABEL) == Some(label))
            .unwrap_or_else(|| panic!("no field {label} in {section}"))
    }

    fn type_into(&mut self, field: NodeId, text: &str) {
        self.tree.set_local_prop(field, prop::VALUE, text.into());
        self.fire(field, event::CHANGE, text.into());
    }

    fn press(&mut self, label: &str) {
        let b = self.find(ControlKind::Button, |n| n.str_prop(prop::LABEL) == Some(label));
        assert_eq!(b.len(), 1, "one button {label}");
        self.fire(b[0], event::PRESS, Value::Null);
    }

    fn search(&mut self, text: &str) {
        let field = self.field("Search", "Search");
        self.type_into(field, text);
    }

    fn row(&self, title: &str) -> NodeId {
        self.find(ControlKind::Row, |n| n.str_prop(prop::TITLE) == Some(title))[0]
    }

    /// The text over the list: "<shown> of <all> notes", or a problem.
    fn status(&self) -> String {
        let s = self.section("All notes").unwrap();
        let text = self.tree.get(s).unwrap().children[0];
        self.tree.get(text).unwrap().text.clone().unwrap_or_default()
    }

    fn rows(&self) -> Vec<String> {
        let list = self.find(ControlKind::List, |_| true)[0];
        self.tree
            .get(list)
            .unwrap()
            .children
            .iter()
            .filter_map(|r| self.tree.get(*r))
            .filter(|n| n.kind == Some(ControlKind::Row))
            .map(|n| n.str_prop(prop::TITLE).unwrap_or_default().to_owned())
            .collect()
    }

    fn add(&mut self, title: &str, body: &str) {
        let t = self.field("Add note", "Title");
        self.type_into(t, title);
        let b = self.field("Add note", "Body");
        self.type_into(b, body);
        self.press("Add note");
    }
}

#[test]
fn add_edit_delete_search_and_keep_the_notes() {
    let data = data_dir("crud");
    let mut app = Notes::start(&data);
    app.wait_rows("the first notes", &FIRST);

    // A new note is on top: the list is sorted by the last change.
    app.add("Groceries", "Bananas and milk");
    app.wait_rows("add", &["Groceries", "Welcome", "Shopping list", "Idea"]);

    // An edit moves the note to the top.
    app.fire(app.row("Idea"), event::PRESS, Value::Null);
    assert!(app.section("Edit note").is_some());
    let title = app.field("Edit note", "Title");
    app.type_into(title, "Big idea");
    app.press("Save");
    app.wait_rows("edit", &["Big idea", "Groceries", "Welcome", "Shopping list"]);
    assert!(app.section("Edit note").is_none());

    // A prefix search on each word, in the title and the body, with no case.
    app.search("mil");
    app.wait_rows("search mil", &["Groceries", "Shopping list"]);
    assert_eq!(app.status(), "2 of 4 notes");
    app.search("MIL egg");
    app.wait_rows("search two words", &["Shopping list"]);
    app.search("calcul");
    app.wait_rows("search the body of an edited note", &["Big idea"]);
    app.search("zzzz");
    app.wait_rows("no match", &[]);
    let empty = app.find(ControlKind::Empty, |_| true)[0];
    assert_eq!(app.tree.get(empty).unwrap().str_prop(prop::TITLE), Some("No matches"));

    // A delete during a search updates the search result.
    app.search("mil");
    app.wait_rows("search mil again", &["Groceries", "Shopping list"]);
    let delete = app.tree.get(app.row("Groceries")).unwrap().children[0];
    app.fire(delete, event::PRESS, Value::Null);
    app.wait_rows("delete", &["Shopping list"]);
    app.search("");
    app.wait_rows("no search", &["Big idea", "Welcome", "Shopping list"]);
    drop(app);

    // The next run reads the same database, and does not add the first notes again.
    let mut app = Notes::start(&data);
    app.wait_rows("the next run", &["Big idea", "Welcome", "Shopping list"]);
    drop(app);
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn search_text_with_quotes_and_fts_operators_never_fails() {
    let data = data_dir("escape");
    let mut app = Notes::start(&data);
    app.wait_rows("the first notes", &FIRST);
    // The cases change between a match and no match, so that each wait sees
    // the answer of its own search.
    let shopping: &[&str] = &["Shopping list"];
    let none: &[&str] = &[];
    let cases: [(&str, &[&str]); 18] = [
        ("\"mil", shopping),
        ("\"", none),
        ("mil*", shopping),
        ("*", none),
        ("-milk", shopping),
        ("-", none),
        ("\"milk\" \"eggs\"", shopping),
        ("a\"b", none),
        ("(milk)", shopping),
        ("NEAR(", none),
        ("milk:", shopping),
        ("milk OR zzz", none),
        ("^mil", shopping),
        ("\"\"", none),
        ("'milk'", shopping),
        ("milk AND eggs", none),
        ("coffee.", shopping),
        ("-- ; DROP TABLE notes", none),
    ];
    for (text, rows) in cases {
        app.search(text);
        app.wait_rows(&format!("search {text:?}"), rows);
    }
    app.search("");
    app.wait_rows("the table is still there", &FIRST);
    drop(app);
    let _ = std::fs::remove_dir_all(&data);
}

/// The JSON value of an older build in `store.kv`.
fn old_notes(notes: &[(u32, &str, &str)]) -> String {
    let items: Vec<String> = notes
        .iter()
        .map(|(id, title, body)| format!(r#"{{"id":{id},"title":"{title}","body":"{body}"}}"#))
        .collect();
    format!("[{}]", items.join(","))
}

#[test]
fn the_notes_of_an_older_build_move_from_kv_into_the_table() {
    let data = data_dir("kv");
    let mut kv = Kv::open(&data, ID).unwrap();
    kv.set("notes".into(), old_notes(&[(1, "Old one", "kept in kv"), (7, "Old two", "also old")])).unwrap();
    kv.set("other".into(), "stays".into()).unwrap();
    drop(kv);

    // The old notes, in their old order (the last one is the newest), and no first notes.
    let mut app = Notes::start(&data);
    app.wait_rows("the moved notes", &["Old two", "Old one"]);
    app.search("kv");
    app.wait_rows("search a moved note", &["Old one"]);
    drop(app);
    let kv = Kv::open(&data, ID).unwrap();
    assert_eq!(kv.get("notes"), None, "the old key is removed");
    assert_eq!(kv.get("other").as_deref(), Some("stays"));
    drop(kv);

    // The table has notes: an old value is not copied again (only removed).
    let mut kv = Kv::open(&data, ID).unwrap();
    kv.set("notes".into(), old_notes(&[(1, "Old one", "kept in kv")])).unwrap();
    drop(kv);
    let mut app = Notes::start(&data);
    app.wait_rows("no second copy", &["Old two", "Old one"]);
    drop(app);
    assert_eq!(Kv::open(&data, ID).unwrap().get("notes"), None);

    // An old value with no notes: an empty table, and no first notes.
    let data2 = data_dir("kv-empty");
    let mut kv = Kv::open(&data2, ID).unwrap();
    kv.set("notes".into(), "[]".into()).unwrap();
    drop(kv);
    let mut app = Notes::start(&data2);
    app.wait_rows("an empty old list", &[]);
    drop(app);
    let _ = std::fs::remove_dir_all(&data);
    let _ = std::fs::remove_dir_all(&data2);
}

#[test]
fn search_in_1000_notes() {
    // VALIDATION V2: 1,000 notes. The kv move puts them in the table in one batch.
    let data = data_dir("many");
    let notes: Vec<(u32, String, String)> = (1..=1000)
        .map(|i| (i, format!("Note {i}"), format!("topic{} and some more words for the body of note {i}", i % 50)))
        .collect();
    let refs: Vec<(u32, &str, &str)> = notes.iter().map(|(i, t, b)| (*i, t.as_str(), b.as_str())).collect();
    let mut kv = Kv::open(&data, ID).unwrap();
    kv.set("notes".into(), old_notes(&refs)).unwrap();
    drop(kv);

    let start = Instant::now();
    let mut app = Notes::start(&data);
    app.wait("1000 notes", |a| a.rows().len() == 1000);
    eprintln!("notes: start with 1000 notes: {:?}", start.elapsed());
    assert_eq!(app.status(), "1000 of 1000 notes");

    // "topic7" is in 20 notes (i % 50 == 7); "topic7*" also finds nothing else
    // (topic70.. do not exist).
    app.search("topic7");
    let t = app.wait("search topic7", |a| a.rows().len() == 20);
    eprintln!("notes: search in 1000 notes: {t:?} (with the commit to the test tree)");
    assert_eq!(app.rows()[0], "Note 957", "the last change first");
    // "topic1" also finds topic10..topic19: 11 x 20 notes.
    app.search("topic1");
    let t = app.wait("search topic1", |a| a.rows().len() == 220);
    eprintln!("notes: search with 220 hits in 1000 notes: {t:?}");
    // 961 % 50 == 11.
    app.search("topic1 961");
    app.wait_rows("two words", &["Note 961"]);
    drop(app);
    let _ = std::fs::remove_dir_all(&data);
}
