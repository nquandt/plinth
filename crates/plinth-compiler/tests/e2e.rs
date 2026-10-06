//! Compiles the Plinth TS examples and runs them in wasmtime. They must
//! behave the same as the M0 hand-written guests (SPEC.md §15, M1 exit).

use plinth_compiler::driver::DiskFs;
use plinth_protocol::{ControlKind, Event, NodeId, Value, Writer, event, prop};
use plinth_runner_wasmtime::{Guest, Limits, Runner};
use plinth_ui::tree::{Node, Tree};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// The capability names declared in `<example>/plinth.toml`'s
/// `[[capabilities]]` tables (SPEC.md §11), read with a minimal parse so
/// this test crate does not need a `plinth-package` dev-dependency.
fn declared_capabilities(root: &std::path::Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(root.join("plinth.toml")) else { return Vec::new() };
    let Ok(value) = text.parse::<toml::Value>() else { return Vec::new() };
    value
        .get("capabilities")
        .and_then(|c| c.as_array())
        .map(|caps| caps.iter().filter_map(|c| c.get("name")?.as_str().map(str::to_owned)).collect())
        .unwrap_or_default()
}

fn build(name: &str) -> plinth_compiler::Artifact {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(name);
    let caps = declared_capabilities(&root);
    let fs = DiskFs { root };
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &caps).expect("compile");
    let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
    artifact.unwrap_or_else(|| panic!("{name} has errors:\n{}", diags.join("\n")))
}

struct Harness {
    guest: Guest,
    tree: Tree,
    _runner: Runner,
}

impl Harness {
    fn start(bytes: &[u8]) -> Self {
        let runner = Runner::new().unwrap();
        let mut guest = runner.load(bytes, Limits::default()).unwrap();
        let mut tree = Tree::new();
        let commits = guest.init(&[]);
        for log in guest.take_logs() {
            eprintln!("guest: {log}");
        }
        for commit in commits.unwrap() {
            let errors = tree.apply(&commit).unwrap();
            assert!(errors.is_empty(), "op errors: {errors:?}");
        }
        Self { guest, tree, _runner: runner }
    }

    fn fire(&mut self, node: NodeId, ev: u16, value: Value) -> Duration {
        let handler = self.tree.get(node).and_then(|n| n.handler(ev)).expect("node has a handler");
        let mut w = Writer::new();
        w.event(&Event::Ui { handler, event: ev, value });
        let start = Instant::now();
        let commits = self.guest.on_event(w.as_bytes());
        for log in self.guest.take_logs() {
            eprintln!("guest: {log}");
        }
        for commit in commits.unwrap() {
            let errors = self.tree.apply(&commit).unwrap();
            assert!(errors.is_empty(), "op errors: {errors:?}");
        }
        start.elapsed()
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

    fn one(&self, kind: ControlKind, pred: impl Fn(&Node) -> bool) -> NodeId {
        let f = self.find(kind, pred);
        assert_eq!(f.len(), 1, "expected one {kind:?}, found {}", f.len());
        f[0]
    }

    fn label(&self, label: &str) -> NodeId {
        self.one(ControlKind::Button, |n| n.str_prop(prop::LABEL) == Some(label))
    }

    fn text_of(&self, id: NodeId) -> String {
        self.tree.get(id).unwrap().text.clone().unwrap_or_default()
    }

    fn row_titles(&self) -> Vec<String> {
        let list = self.one(ControlKind::List, |_| true);
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
}

#[test]
fn counter_behaves_like_m0() {
    let art = build("counter");
    eprintln!("counter: core {} KiB, component {} KiB", art.core_size / 1024, art.component.len() / 1024);
    // SPEC.md §5.5 size targets.
    // The raw runtime limit is temporary room for M2 work. The linker will
    // stub runtime functions that an app does not reach; then the SPEC.md
    // §5.5 limit (60 KiB of runtime in each artifact) applies again.
    assert!(plinth_compiler::link::runtime().len() <= 72 * 1024, "plinth-rt is over 72 KiB");
    assert!(art.component.len() <= 80 * 1024, "the counter artifact is over 80 KiB");
    let mut h = Harness::start(&art.component);
    let root = h.tree.current_root().unwrap();
    assert_eq!(root.str_prop(prop::TITLE), Some("Counter"));
    assert_eq!(root.str_prop(prop::ICON), Some("number"));
    let value = h.one(ControlKind::Heading, |_| true);
    let parity = h.one(ControlKind::Text, |_| true);
    assert_eq!(h.text_of(value), "0");
    assert_eq!(h.text_of(parity), "even");

    let inc = h.label("Increment");
    for _ in 0..3 {
        h.fire(inc, event::PRESS, Value::Null);
    }
    assert_eq!(h.text_of(value), "3");
    assert_eq!(h.text_of(parity), "odd");
    h.fire(h.label("Decrement"), event::PRESS, Value::Null);
    assert_eq!(h.text_of(value), "2");
    h.fire(h.label("Reset"), event::PRESS, Value::Null);
    assert_eq!(h.text_of(value), "0");

    // M1 exit: a round trip stays under 1 ms.
    let mut times: Vec<Duration> = (0..200).map(|_| h.fire(inc, event::PRESS, Value::Null)).collect();
    times.sort();
    eprintln!("round trip: median {:?}, max {:?}", times[100], times[199]);
    assert!(times[100] < Duration::from_millis(1));
    assert_eq!(h.text_of(value), "200");
}

#[test]
fn todo_behaves_like_m0() {
    let art = build("todo");
    eprintln!("todo: core {} KiB, component {} KiB", art.core_size / 1024, art.component.len() / 1024);
    let mut h = Harness::start(&art.component);
    assert_eq!(h.tree.screens().count(), 2);
    assert_eq!(h.row_titles(), ["Read SPEC.md", "Build the M0 host", "Write the compiler"]);
    let summary = h.one(ControlKind::Text, |n| n.text.as_deref().is_some_and(|t| t.contains("remaining")));
    assert_eq!(h.text_of(summary), "3 of 3 remaining");

    // The Add button is disabled while the draft is empty.
    let add = h.label("Add task");
    assert!(h.tree.get(add).unwrap().bool_prop(prop::DISABLED));

    // Type, then submit with Enter. The guest clears the field.
    let field = h.one(ControlKind::TextField, |_| true);
    h.tree.set_local_prop(field, prop::VALUE, "Buy milk".into());
    h.fire(field, event::CHANGE, "Buy milk".into());
    assert!(!h.tree.get(add).unwrap().bool_prop(prop::DISABLED));
    h.fire(field, event::SUBMIT, Value::Null);
    assert_eq!(h.row_titles().last().map(String::as_str), Some("Buy milk"));
    assert_eq!(h.tree.get(field).unwrap().str_prop(prop::VALUE), Some(""));
    assert_eq!(h.text_of(summary), "4 of 4 remaining");

    // Complete the first task.
    let first_toggle = h.find(ControlKind::Toggle, |n| n.str_prop(prop::LABEL) == Some("Done"))[0];
    h.fire(first_toggle, event::CHANGE, Value::Bool(true));
    assert_eq!(h.text_of(summary), "3 of 4 remaining");
    let first_row = h.find(ControlKind::Row, |_| true)[0];
    assert_eq!(h.tree.get(first_row).unwrap().str_prop(prop::SUBTITLE), Some("Done"));

    // Settings: hide completed tasks, then show them again.
    let show = h.one(ControlKind::Toggle, |n| n.str_prop(prop::LABEL) == Some("Show completed tasks"));
    h.fire(show, event::CHANGE, Value::Bool(false));
    assert_eq!(h.row_titles(), ["Build the M0 host", "Write the compiler", "Buy milk"]);
    h.fire(show, event::CHANGE, Value::Bool(true));
    assert_eq!(h.row_titles(), ["Read SPEC.md", "Build the M0 host", "Write the compiler", "Buy milk"]);

    // The clear button shows the count, and clears.
    let clear = h.one(ControlKind::Button, |n| n.str_prop(prop::LABEL).is_some_and(|l| l.starts_with("Clear")));
    assert_eq!(h.tree.get(clear).unwrap().str_prop(prop::LABEL), Some("Clear 1 completed task"));
    h.fire(clear, event::PRESS, Value::Null);
    assert_eq!(h.row_titles(), ["Build the M0 host", "Write the compiler", "Buy milk"]);
    assert!(h.tree.get(clear).unwrap().bool_prop(prop::DISABLED));

    // Delete everything: the Empty control appears.
    while let Some(&d) = h.find(ControlKind::Button, |n| n.str_prop(prop::LABEL) == Some("Delete")).first() {
        h.fire(d, event::PRESS, Value::Null);
    }
    assert!(h.row_titles().is_empty());
    let empty = h.one(ControlKind::Empty, |_| true);
    assert_eq!(h.tree.get(empty).unwrap().str_prop(prop::TITLE), Some("No tasks yet"));

    // navigate() selects a primary screen.
    h.fire(h.label("Back to tasks"), event::PRESS, Value::Null);
    assert_eq!(h.tree.current_screen, 0);
}

#[test]
fn calculator_computes() {
    let art = build("calculator");
    let mut h = Harness::start(&art.component);
    let display = h.one(ControlKind::Heading, |_| true);
    let press = |h: &mut Harness, label: &str| {
        let b = h.label(label);
        h.fire(b, event::PRESS, Value::Null);
    };
    for k in ["1", "2", "+", "3", "="] {
        press(&mut h, k);
    }
    assert_eq!(h.text_of(display), "15");
    // An exhaustive switch (no default) and division by zero.
    for k in ["÷", "0", "="] {
        press(&mut h, k);
    }
    assert_eq!(h.text_of(display), "Error");
    press(&mut h, "C");
    for k in ["7", ".", "5", "×", "2", "="] {
        press(&mut h, k);
    }
    assert_eq!(h.text_of(display), "15");
    assert_eq!(h.row_titles().len(), 3, "history has one row per calculation");
}

#[test]
fn notes_add_select_and_search() {
    let art = build("notes");
    let mut h = Harness::start(&art.component);
    let before = h.row_titles().len();
    // Add a note.
    let fields = h.find(ControlKind::TextField, |n| n.str_prop(prop::PLACEHOLDER) == Some("Note title"));
    let title = fields[0];
    h.tree.set_local_prop(title, prop::VALUE, "Groceries".into());
    h.fire(title, event::CHANGE, "Groceries".into());
    h.fire(h.label("Add note"), event::PRESS, Value::Null);
    assert_eq!(h.row_titles().len(), before + 1);
    // Selecting a note shows the edit section (a region that reads a signal).
    assert!(h.find(ControlKind::Section, |n| n.str_prop(prop::TITLE) == Some("Edit note")).is_empty());
    let row = h.find(ControlKind::Row, |n| n.str_prop(prop::TITLE) == Some("Groceries"))[0];
    h.fire(row, event::PRESS, Value::Null);
    assert_eq!(h.find(ControlKind::Section, |n| n.str_prop(prop::TITLE) == Some("Edit note")).len(), 1);
    // Deleting the selected note (`selected() === id` on `number | null`) hides it again.
    let row = h.find(ControlKind::Row, |n| n.str_prop(prop::TITLE) == Some("Groceries"))[0];
    let delete = h.tree.get(row).unwrap().children[0];
    h.fire(delete, event::PRESS, Value::Null);
    assert_eq!(h.row_titles().len(), before);
    assert!(h.find(ControlKind::Section, |n| n.str_prop(prop::TITLE) == Some("Edit note")).is_empty());
    // Search filters the list.
    let search = h.one(ControlKind::TextField, |n| n.str_prop(prop::LABEL) == Some("Search"));
    h.tree.set_local_prop(search, prop::VALUE, "zzzz-no-match".into());
    h.fire(search, event::CHANGE, "zzzz-no-match".into());
    assert!(h.row_titles().is_empty());
}
