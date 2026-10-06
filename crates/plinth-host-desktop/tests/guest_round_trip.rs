//! Runs the M0 example guests in wasmtime and drives them with events,
//! without a window. Build the guests first: `cargo run -p plinth-cli -- example todo-rs`
//! (and `counter-rs`). The tests skip when an artifact is missing.

use plinth_protocol::{ControlKind, Event, NodeId, Value, Writer, event, prop};
use plinth_runner_wasmtime::{Guest, Limits, Runner};
use plinth_ui::tree::Tree;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn artifact(name: &str) -> Option<Vec<u8>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/plinth").join(format!("{name}.wasm"));
    match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(_) => {
            eprintln!("skip: {} is missing; run `cargo run -p plinth-cli -- example {name}`", path.display());
            None
        }
    }
}

struct Harness {
    guest: Guest,
    tree: Tree,
    _runner: std::sync::Arc<Runner>,
}

impl Harness {
    fn start(bytes: &[u8]) -> Self {
        Self::start_on(std::sync::Arc::new(Runner::new().unwrap()), bytes)
    }

    /// Like `start`, on a `Runner` the caller already has (so several
    /// harnesses can share one engine, `docs/HUB.md` §4.2, §12.2).
    fn start_on(runner: std::sync::Arc<Runner>, bytes: &[u8]) -> Self {
        let mut guest = runner.load(bytes, Limits::default()).unwrap();
        let mut tree = Tree::new();
        for commit in guest.init(&[]).unwrap() {
            assert!(tree.apply(&commit).unwrap().is_empty());
        }
        Self { guest, tree, _runner: runner }
    }

    /// Sends one event and returns the round-trip time (dispatch + apply).
    fn fire(&mut self, node: NodeId, ev: u16, value: Value) -> Duration {
        let handler = self.tree.get(node).and_then(|n| n.handler(ev)).expect("node has a handler");
        let mut w = Writer::new();
        w.event(&Event::Ui { handler, event: ev, value });
        let start = Instant::now();
        for commit in self.guest.on_event(w.as_bytes()).unwrap() {
            let errors = self.tree.apply(&commit).unwrap();
            assert!(errors.is_empty(), "op errors: {errors:?}");
        }
        start.elapsed()
    }

    fn find(&self, kind: ControlKind, pred: impl Fn(&plinth_ui::tree::Node) -> bool) -> Vec<NodeId> {
        let mut found = Vec::new();
        let roots: Vec<NodeId> = self.tree.screens().map(|(_, id)| id).collect();
        let mut stack = roots;
        while let Some(id) = stack.pop() {
            let node = self.tree.get(id).unwrap();
            if node.kind == Some(kind) && pred(node) {
                found.push(id);
            }
            stack.extend(node.children.iter().rev());
        }
        found
    }

    fn find_one(&self, kind: ControlKind, pred: impl Fn(&plinth_ui::tree::Node) -> bool) -> NodeId {
        let found = self.find(kind, pred);
        assert_eq!(found.len(), 1, "expected one {kind:?}");
        found[0]
    }

    fn row_titles(&self) -> Vec<String> {
        let list = self.find_one(ControlKind::List, |_| true);
        self.tree
            .get(list)
            .unwrap()
            .children
            .iter()
            .map(|r| self.tree.get(*r).unwrap().str_prop(prop::TITLE).unwrap().to_owned())
            .collect()
    }
}

#[test]
fn counter_increments() {
    let Some(bytes) = artifact("counter-rs") else { return };
    let mut h = Harness::start(&bytes);
    let inc = h.find_one(ControlKind::Button, |n| n.str_prop(prop::LABEL) == Some("Increment"));
    let value = h.find_one(ControlKind::Heading, |_| true);
    for _ in 0..3 {
        h.fire(inc, event::PRESS, Value::Null);
    }
    assert_eq!(h.tree.get(value).unwrap().text.as_deref(), Some("3"));
    let reset = h.find_one(ControlKind::Button, |n| n.str_prop(prop::LABEL) == Some("Reset"));
    h.fire(reset, event::PRESS, Value::Null);
    assert_eq!(h.tree.get(value).unwrap().text.as_deref(), Some("0"));
}

#[test]
fn todo_add_complete_filter_delete() {
    let Some(bytes) = artifact("todo-rs") else { return };
    let mut h = Harness::start(&bytes);
    assert_eq!(h.tree.screens().count(), 2);
    assert_eq!(h.row_titles(), ["Read SPEC.md", "Build the M0 host", "Write the compiler"]);

    // Type a task and submit it with Enter.
    let field = h.find_one(ControlKind::TextField, |_| true);
    h.tree.set_local_prop(field, prop::VALUE, "Buy milk".into());
    h.fire(field, event::CHANGE, "Buy milk".into());
    h.fire(field, event::SUBMIT, "Buy milk".into());
    assert_eq!(h.row_titles().last().map(String::as_str), Some("Buy milk"));
    // The guest clears the field.
    assert_eq!(h.tree.get(field).unwrap().str_prop(prop::VALUE), Some(""));

    // Complete the first task, then hide completed tasks.
    let first_toggle = h.find(ControlKind::Toggle, |n| n.str_prop(prop::LABEL) == Some("Done"))[0];
    h.fire(first_toggle, event::CHANGE, Value::Bool(true));
    let show = h.find_one(ControlKind::Toggle, |n| n.str_prop(prop::LABEL) == Some("Show completed tasks"));
    h.fire(show, event::CHANGE, Value::Bool(false));
    assert_eq!(h.row_titles(), ["Build the M0 host", "Write the compiler", "Buy milk"]);

    // Show it again. The keyed reconciler puts the row back in its place.
    h.fire(show, event::CHANGE, Value::Bool(true));
    assert_eq!(h.row_titles(), ["Read SPEC.md", "Build the M0 host", "Write the compiler", "Buy milk"]);

    // Delete all tasks. The Empty control appears.
    while let Some(&delete) = h.find(ControlKind::Button, |n| n.str_prop(prop::LABEL) == Some("Delete")).first() {
        h.fire(delete, event::PRESS, Value::Null);
    }
    assert!(h.row_titles().is_empty());
    h.find_one(ControlKind::Empty, |_| true);
}

/// SPEC.md §15 M0 exit: an event makes a round trip in under 1 ms.
#[test]
fn event_round_trip_is_under_one_millisecond() {
    let Some(bytes) = artifact("counter-rs") else { return };
    let mut h = Harness::start(&bytes);
    let inc = h.find_one(ControlKind::Button, |n| n.str_prop(prop::LABEL) == Some("Increment"));
    h.fire(inc, event::PRESS, Value::Null); // warm up
    let mut times: Vec<Duration> = (0..200).map(|_| h.fire(inc, event::PRESS, Value::Null)).collect();
    times.sort();
    let median = times[times.len() / 2];
    eprintln!("round trip: median {median:?}, max {:?}", times.last().unwrap());
    assert!(median < Duration::from_millis(1), "median round trip {median:?}");
}

#[test]
fn imports_outside_the_world_are_rejected() {
    // A component that imports `wasi:cli/environment` must not load.
    let wat = r#"(component (import "wasi:cli/environment@0.2.0" (instance)))"#;
    let bytes = wat_to_component(wat);
    let runner = Runner::new().unwrap();
    let err = runner.load(&bytes, Limits::default()).err().expect("load must fail");
    assert!(format!("{err:#}").contains("not in the plinth:app world"), "{err:#}");
}

fn wat_to_component(wat: &str) -> Vec<u8> {
    wat::parse_str(wat).unwrap()
}

/// `docs/HUB.md` §4.2, §12.2: several apps run in one host process, each
/// with its own guest instance, sharing one `Runner`/engine. This checks
/// the non-gpui half of that (the window and `open_app` need a running
/// `gpui::App`, checked instead by the live `plinth hub run` smoke test).
#[test]
fn two_guests_share_one_runner_and_stay_independent() {
    let Some(bytes) = artifact("counter-rs") else { return };
    let runner = std::sync::Arc::new(Runner::new().unwrap());

    let mut a = Harness::start_on(runner.clone(), &bytes);
    let mut b = Harness::start_on(runner.clone(), &bytes);

    let inc_a = a.find_one(ControlKind::Button, |n| n.str_prop(prop::LABEL) == Some("Increment"));
    let value_a = a.find_one(ControlKind::Heading, |_| true);
    let inc_b = b.find_one(ControlKind::Button, |n| n.str_prop(prop::LABEL) == Some("Increment"));
    let value_b = b.find_one(ControlKind::Heading, |_| true);

    a.fire(inc_a, event::PRESS, Value::Null);
    a.fire(inc_a, event::PRESS, Value::Null);
    b.fire(inc_b, event::PRESS, Value::Null);

    // Each guest kept its own state: dispatching to `a` did not affect `b`.
    assert_eq!(a.tree.get(value_a).unwrap().text.as_deref(), Some("2"));
    assert_eq!(b.tree.get(value_b).unwrap().text.as_deref(), Some("1"));
}
