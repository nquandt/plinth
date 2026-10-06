//! GC stress mode (SPEC.md §5.4, §16): forces a full collection after every
//! `init` and `on-event`, so the usual e2e flows and an allocation-heavy app
//! run with the collector exercised continuously instead of only once the
//! idle threshold is crossed. A bug in root tracing or object layout then
//! shows up as a wrong read (or a crash), not as a slow leak.
//!
//! Stress mode is turned on through `init`'s `args` byte 0 (`== 1`), which
//! `plinth-rt`'s `Rt::init` reads before anything else (see `lib.rs`).

use plinth_compiler::driver::DiskFs;
use plinth_protocol::{ControlKind, Event, NodeId, Value, Writer, event, prop};
use plinth_runner_wasmtime::{Guest, Limits, Runner};
use plinth_ui::tree::{Node, Tree};
use std::path::PathBuf;
use std::time::Instant;

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
    /// Starts the guest with GC stress mode on: it collects after `init`
    /// and after every `on-event` for the rest of this instance's life.
    fn start_stress(bytes: &[u8]) -> Self {
        let runner = Runner::new().unwrap();
        let mut guest = runner.load(bytes, Limits::default()).unwrap();
        let mut tree = Tree::new();
        let commits = guest.init(&[1]);
        for log in guest.take_logs() {
            eprintln!("guest: {log}");
        }
        for commit in commits.unwrap() {
            let errors = tree.apply(&commit).unwrap();
            assert!(errors.is_empty(), "op errors: {errors:?}");
        }
        Self { guest, tree, _runner: runner }
    }

    fn fire(&mut self, node: NodeId, ev: u16, value: Value) {
        let handler = self.tree.get(node).and_then(|n| n.handler(ev)).expect("node has a handler");
        let mut w = Writer::new();
        w.event(&Event::Ui { handler, event: ev, value });
        let commits = self.guest.on_event(w.as_bytes());
        for log in self.guest.take_logs() {
            eprintln!("guest: {log}");
        }
        for commit in commits.unwrap() {
            let errors = self.tree.apply(&commit).unwrap();
            assert!(errors.is_empty(), "op errors: {errors:?}");
        }
    }

    fn fire_timers(&mut self, now: Instant) {
        let commits = self.guest.fire_due_timers(now).unwrap();
        for log in self.guest.take_logs() {
            eprintln!("guest: {log}");
        }
        for commit in commits {
            let errors = self.tree.apply(&commit).unwrap();
            assert!(errors.is_empty(), "op errors: {errors:?}");
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
fn todo_under_stress() {
    let art = build("todo");
    let mut h = Harness::start_stress(&art.component);
    assert_eq!(h.row_titles(), ["Read SPEC.md", "Build the M0 host", "Write the compiler"]);

    let add = h.label("Add task");
    let field = h.one(ControlKind::TextField, |_| true);
    h.tree.set_local_prop(field, prop::VALUE, "Buy milk".into());
    h.fire(field, event::CHANGE, "Buy milk".into());
    assert!(!h.tree.get(add).unwrap().bool_prop(prop::DISABLED));
    h.fire(field, event::SUBMIT, Value::Null);
    assert_eq!(h.row_titles().last().map(String::as_str), Some("Buy milk"));

    let first_toggle = h.find(ControlKind::Toggle, |n| n.str_prop(prop::LABEL) == Some("Done"))[0];
    h.fire(first_toggle, event::CHANGE, Value::Bool(true));

    let show = h.one(ControlKind::Toggle, |n| n.str_prop(prop::LABEL) == Some("Show completed tasks"));
    h.fire(show, event::CHANGE, Value::Bool(false));
    assert_eq!(h.row_titles(), ["Build the M0 host", "Write the compiler", "Buy milk"]);
    h.fire(show, event::CHANGE, Value::Bool(true));
    assert_eq!(h.row_titles(), ["Read SPEC.md", "Build the M0 host", "Write the compiler", "Buy milk"]);

    let clear = h.one(ControlKind::Button, |n| n.str_prop(prop::LABEL).is_some_and(|l| l.starts_with("Clear")));
    h.fire(clear, event::PRESS, Value::Null);
    assert_eq!(h.row_titles(), ["Build the M0 host", "Write the compiler", "Buy milk"]);

    while let Some(&d) = h.find(ControlKind::Button, |n| n.str_prop(prop::LABEL) == Some("Delete")).first() {
        h.fire(d, event::PRESS, Value::Null);
    }
    assert!(h.row_titles().is_empty());
    let empty = h.one(ControlKind::Empty, |_| true);
    assert_eq!(h.tree.get(empty).unwrap().str_prop(prop::TITLE), Some("No tasks yet"));

    h.fire(h.label("Back to tasks"), event::PRESS, Value::Null);
    assert_eq!(h.tree.current_screen, 0);
}

#[test]
fn notes_under_stress() {
    let art = build("notes");
    let mut h = Harness::start_stress(&art.component);
    let before = h.row_titles().len();

    let fields = h.find(ControlKind::TextField, |n| n.str_prop(prop::PLACEHOLDER) == Some("Note title"));
    let title = fields[0];
    h.tree.set_local_prop(title, prop::VALUE, "Groceries".into());
    h.fire(title, event::CHANGE, "Groceries".into());
    h.fire(h.label("Add note"), event::PRESS, Value::Null);
    assert_eq!(h.row_titles().len(), before + 1);

    let row = h.find(ControlKind::Row, |n| n.str_prop(prop::TITLE) == Some("Groceries"))[0];
    h.fire(row, event::PRESS, Value::Null);
    assert_eq!(h.find(ControlKind::Section, |n| n.str_prop(prop::TITLE) == Some("Edit note")).len(), 1);

    let row = h.find(ControlKind::Row, |n| n.str_prop(prop::TITLE) == Some("Groceries"))[0];
    let delete = h.tree.get(row).unwrap().children[0];
    h.fire(delete, event::PRESS, Value::Null);
    assert_eq!(h.row_titles().len(), before);
    assert!(h.find(ControlKind::Section, |n| n.str_prop(prop::TITLE) == Some("Edit note")).is_empty());

    let search = h.one(ControlKind::TextField, |n| n.str_prop(prop::LABEL) == Some("Search"));
    h.tree.set_local_prop(search, prop::VALUE, "zzzz-no-match".into());
    h.fire(search, event::CHANGE, "zzzz-no-match".into());
    assert!(h.row_titles().is_empty());
}

#[test]
fn contacts_under_stress() {
    let art = build("contacts");
    let mut h = Harness::start_stress(&art.component);
    assert_eq!(h.row_titles(), ["Ada Lovelace", "Grace Hopper", "Alan Turing"]);

    let tabs = h.one(ControlKind::Tabs, |_| true);
    h.fire(tabs, event::CHANGE, "favorites".into());
    assert_eq!(h.row_titles(), ["Ada Lovelace", "Grace Hopper"]);
    h.fire(tabs, event::CHANGE, "all".into());

    let row = h.find(ControlKind::Row, |n| n.str_prop(prop::TITLE) == Some("Ada Lovelace"))[0];
    h.fire(row, event::PRESS, Value::Null);
    let detail = h.tree.current_root().unwrap();
    assert_eq!(detail.str_prop(prop::TITLE), Some("Ada Lovelace"));

    let edit = h.one(ControlKind::Action, |n| n.str_prop(prop::LABEL) == Some("Edit"));
    h.fire(edit, event::PRESS, Value::Null);
    let sheet = h.one(ControlKind::Sheet, |_| true);
    let name_field = h.tree.get(sheet).unwrap().children[0];
    h.tree.set_local_prop(name_field, prop::VALUE, "Ada, Countess Lovelace".into());
    h.fire(name_field, event::CHANGE, "Ada, Countess Lovelace".into());
    h.fire(h.label("Save"), event::PRESS, Value::Null);
    assert_eq!(h.tree.current_root().unwrap().str_prop(prop::TITLE), Some("Ada, Countess Lovelace"));

    let delete = h.one(ControlKind::Action, |n| n.str_prop(prop::LABEL) == Some("Delete"));
    h.fire(delete, event::PRESS, Value::Null);
    assert!(!h.tree.can_go_back());
    assert_eq!(h.tree.current_root().unwrap().str_prop(prop::TITLE), Some("Contacts"));
    assert_eq!(h.row_titles(), ["Grace Hopper", "Alan Turing"]);
}

#[test]
fn timer_under_stress() {
    use std::time::Duration;

    let art = build("timer");
    let mut h = Harness::start_stress(&art.component);
    let heading = h.one(ControlKind::Heading, |_| true);
    assert_eq!(h.text_of(heading), "0.0s");

    h.fire(h.label("Start"), event::PRESS, Value::Null);
    let t0 = Instant::now();
    h.fire_timers(t0 + Duration::from_millis(100));
    assert_eq!(h.text_of(heading), "0.1s");
    h.fire_timers(t0 + Duration::from_millis(250));
    assert_eq!(h.text_of(heading), "0.2s");

    h.fire(h.label("Stop"), event::PRESS, Value::Null);
    h.fire_timers(t0 + Duration::from_secs(5));
    assert_eq!(h.text_of(heading), "0.2s");

    h.fire(h.label("Reset"), event::PRESS, Value::Null);
    assert_eq!(h.text_of(heading), "0.0s");
}

#[test]
fn settings_gallery_under_stress() {
    let art = build("settings-gallery");
    let mut h = Harness::start_stress(&art.component);

    let checkbox = h.one(ControlKind::Checkbox, |n| n.str_prop(prop::LABEL) == Some("Email notifications"));
    h.tree.set_local_prop(checkbox, prop::VALUE, Value::Bool(false));
    h.fire(checkbox, event::CHANGE, Value::Bool(false));
    assert!(!h.tree.get(checkbox).unwrap().bool_prop(prop::VALUE));

    let bio = h.one(ControlKind::TextArea, |_| true);
    h.tree.set_local_prop(bio, prop::VALUE, "Loves Rust".into());
    h.fire(bio, event::CHANGE, "Loves Rust".into());
    assert_eq!(h.tree.get(bio).unwrap().str_prop(prop::VALUE), Some("Loves Rust"));

    let volume = h.one(ControlKind::Slider, |n| n.str_prop(prop::LABEL) == Some("Volume"));
    h.tree.set_local_prop(volume, prop::VALUE, Value::Number(65.0));
    h.fire(volume, event::CHANGE, Value::Number(65.0));
    let progress = h.one(ControlKind::Progress, |n| n.str_prop(prop::LABEL) == Some("Sync progress"));
    assert_eq!(h.tree.get(progress).unwrap().num_prop(prop::VALUE), Some(0.65));

    let theme = h.one(ControlKind::Picker, |n| n.str_prop(prop::LABEL) == Some("Theme"));
    h.tree.set_local_prop(theme, prop::VALUE, "dark".into());
    h.fire(theme, event::CHANGE, "dark".into());
    assert_eq!(h.tree.get(theme).unwrap().str_prop(prop::VALUE), Some("dark"));

    let plan = h.one(ControlKind::Picker, |n| n.str_prop(prop::LABEL) == Some("Plan"));
    h.tree.set_local_prop(plan, prop::VALUE, "pro".into());
    h.fire(plan, event::CHANGE, "pro".into());
    let badge = h.one(ControlKind::Badge, |_| true);
    assert_eq!(h.tree.get(badge).unwrap().str_prop(prop::LABEL), Some("pro"));
}

/// An allocation-heavy app: string concatenation in a loop, a `Map`/`Set`
/// built from scratch on every recompute, an array of structs, a closure
/// captured per list row, and a region that switches content. Run with the
/// collector forced after every event, this is where a missing root or a
/// freed-too-early bug is most likely to surface as wrong output.
#[test]
fn allocation_heavy_app_under_stress() {
    let art = build("gc-torture");
    let mut h = Harness::start_stress(&art.component);

    let text_count = |h: &Harness| -> usize {
        let t = h.one(ControlKind::Text, |_| true);
        h.text_of(t).trim_start_matches("items ").parse().unwrap()
    };
    assert_eq!(text_count(&h), 24);
    assert_eq!(h.row_titles().len(), 24);

    // Flip the region between Text and Heading several times; each flip
    // disposes one scope's heap-referencing state and builds the other's.
    for _ in 0..5 {
        h.fire(h.label("Flip"), event::PRESS, Value::Null);
        h.fire(h.label("Flip"), event::PRESS, Value::Null);
    }
    assert_eq!(text_count(&h), 24);

    // Grow: every row is replaced (the item array and every label string
    // change), many times over, with a collection after each.
    for n in 1..=30 {
        h.fire(h.label("Grow"), event::PRESS, Value::Null);
        assert_eq!(h.row_titles().len(), 24, "iteration {n}");
        let longest = h.row_titles().into_iter().map(|t| t.len()).max().unwrap();
        assert!(longest > 0);
    }

    // Exercise the row closures (each captures a distinct struct from the
    // last build) after many collections. Bumping changes the seed, which
    // changes every row's key, so the list fully rebuilds after each press;
    // re-find the buttons each time rather than firing a stale node id.
    for _ in 0..24 {
        let b = h.find(ControlKind::Button, |n| n.str_prop(prop::LABEL) == Some("Bump"))[0];
        h.fire(b, event::PRESS, Value::Null);
    }
    assert_eq!(h.row_titles().len(), 24);
}
