//! 7GUIs tasks 1-5 (docs/VALIDATION.md V1): compiles each app in
//! `examples/7guis/` and drives it in wasmtime, as `tests/apps.rs` does for
//! the other examples. `web/test/run-7guis.mjs` checks the same behavior on
//! the web host.

use plinth_compiler::driver::DiskFs;
use plinth_protocol::{ControlKind, Event, NodeId, Value, Writer, event, prop};
use plinth_runner_wasmtime::{Guest, Limits, Runner};
use plinth_ui::tree::{Node, Tree};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn build(name: &str) -> plinth_compiler::Artifact {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/7guis").join(name);
    let fs = DiskFs { root };
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[]).expect("compile");
    let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
    let art = artifact.unwrap_or_else(|| panic!("{name} has errors:\n{}", diags.join("\n")));
    eprintln!("7guis/{name}: app {} B, component {} B", art.app.len(), art.component.len());
    art
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
        let mut h = Self { guest, tree, _runner: runner };
        h.check_errors();
        h
    }

    fn check_errors(&mut self) {
        let errors = self.guest.take_errors();
        assert!(errors.is_empty(), "the app reported errors: {errors:?}");
    }

    fn apply(&mut self, commits: Vec<Vec<u8>>) {
        for log in self.guest.take_logs() {
            eprintln!("guest: {log}");
        }
        for commit in commits {
            let errors = self.tree.apply(&commit).unwrap();
            assert!(errors.is_empty(), "op errors: {errors:?}");
        }
        self.check_errors();
    }

    /// Sends one UI event, as a host does. Returns the ops the guest sent
    /// back, so a test can check what changed.
    fn fire(&mut self, node: NodeId, ev: u16, value: Value) -> Vec<Vec<u8>> {
        let handler = self.tree.get(node).and_then(|n| n.handler(ev)).expect("node has a handler");
        let mut w = Writer::new();
        w.event(&Event::Ui { handler, event: ev, value });
        let commits = self.guest.on_event(w.as_bytes()).unwrap();
        self.apply(commits.clone());
        commits
    }

    /// A `change` of a text field, as a host sends it: the host keeps the
    /// typed text in its tree (the guest does not echo it back).
    fn type_text(&mut self, node: NodeId, text: &str) -> Vec<Vec<u8>> {
        self.tree.set_local_prop(node, prop::VALUE, Value::Str(text.into()));
        self.fire(node, event::CHANGE, text.into())
    }

    fn press(&mut self, label: &str) {
        let b = self.button(label);
        assert!(!self.node(b).bool_prop(prop::DISABLED), "{label} is disabled");
        self.fire(b, event::PRESS, Value::Null);
    }

    fn fire_timers(&mut self, now: Instant) {
        let commits = self.guest.fire_due_timers(now).unwrap();
        self.apply(commits);
    }

    fn node(&self, id: NodeId) -> &Node {
        self.tree.get(id).unwrap()
    }

    fn find(&self, kind: ControlKind, pred: impl Fn(&Node) -> bool) -> Vec<NodeId> {
        let mut found = Vec::new();
        let Some(root) = self.tree.current_root() else { return found };
        let mut stack = vec![root.id];
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

    fn labeled(&self, kind: ControlKind, label: &str) -> NodeId {
        self.one(kind, |n| n.str_prop(prop::LABEL) == Some(label))
    }

    fn button(&self, label: &str) -> NodeId {
        self.labeled(ControlKind::Button, label)
    }

    fn field(&self, label: &str) -> NodeId {
        self.labeled(ControlKind::TextField, label)
    }

    fn value(&self, id: NodeId) -> String {
        self.node(id).str_prop(prop::VALUE).unwrap_or_default().to_owned()
    }

    fn error(&self, id: NodeId) -> String {
        self.node(id).str_prop(prop::ERROR).unwrap_or_default().to_owned()
    }

    fn disabled(&self, label: &str) -> bool {
        self.node(self.button(label)).bool_prop(prop::DISABLED)
    }

    fn texts(&self) -> Vec<String> {
        self.find(ControlKind::Text, |_| true).into_iter().map(|id| self.node(id).text.clone().unwrap_or_default()).collect()
    }

    fn heading(&self) -> String {
        let h = self.one(ControlKind::Heading, |_| true);
        self.node(h).text.clone().unwrap_or_default()
    }

    fn rows(&self) -> Vec<NodeId> {
        self.find(ControlKind::Row, |_| true)
    }

    fn row_titles(&self) -> Vec<String> {
        self.rows().into_iter().map(|r| self.node(r).str_prop(prop::TITLE).unwrap_or_default().to_owned()).collect()
    }
}

/// Whether a commit sets the `value` prop of `node`.
fn sets_value(commits: &[Vec<u8>], node: NodeId) -> bool {
    commits.iter().any(|c| {
        plinth_protocol::decode_ops(c).unwrap().iter().any(
            |op| matches!(op, plinth_protocol::Op::SetProp { id, prop: p, .. } if *id == node && *p == prop::VALUE),
        )
    })
}

#[test]
fn counter_counts() {
    let art = build("counter");
    let mut h = Harness::start(&art.component);
    assert_eq!(h.heading(), "0");
    for _ in 0..3 {
        h.press("Count");
    }
    assert_eq!(h.heading(), "3");
}

#[test]
fn temperature_converts_both_ways_and_keeps_invalid_input() {
    let art = build("temperature");
    let mut h = Harness::start(&art.component);
    let c = h.field("Celsius");
    let f = h.field("Fahrenheit");
    assert_eq!((h.value(c), h.value(f)), (String::new(), String::new()));

    // Celsius to Fahrenheit. The guest sets the other field, and does not
    // send back the text that the user typed (no loop, no caret jump).
    let ops = h.type_text(c, "100");
    assert_eq!(h.value(f), "212");
    assert!(sets_value(&ops, f));
    assert!(!sets_value(&ops, c), "the typed field got its own value back");
    h.type_text(c, "-40");
    assert_eq!(h.value(f), "-40");
    h.type_text(c, "37.5");
    assert_eq!(h.value(f), "99.5");

    // Fahrenheit to Celsius.
    h.type_text(f, "32");
    assert_eq!(h.value(c), "0");
    h.type_text(f, "98.6");
    assert_eq!(h.value(c), "37");
    assert_eq!(h.error(c), "");
    assert_eq!(h.error(f), "");

    // Text that is not a number stays, shows an error, and does not change
    // the other field.
    let ops = h.type_text(c, "37a");
    assert_eq!(h.value(c), "37a");
    assert_eq!(h.value(f), "98.6");
    assert_eq!(h.error(c), "Not a number");
    assert!(!sets_value(&ops, c) && !sets_value(&ops, f));
    // An empty field is not an error and changes nothing.
    h.type_text(c, "");
    assert_eq!((h.value(c).as_str(), h.value(f).as_str(), h.error(c).as_str()), ("", "98.6", ""));

    // A valid number in the other field replaces the invalid text.
    h.type_text(c, "abc");
    h.type_text(f, "50");
    assert_eq!(h.value(c), "10");
    assert_eq!(h.error(c), "");
}

#[test]
fn flight_booker_constraints() {
    let art = build("flight-booker");
    let mut h = Harness::start(&art.component);
    let start = h.field("Start date");
    // The start date is today, so a one-way flight can be booked at once.
    assert_eq!(h.value(start).len(), 10, "{}", h.value(start));
    assert!(!h.disabled("Book"));
    // One-way: no return date field.
    assert!(h.find(ControlKind::TextField, |n| n.str_prop(prop::LABEL) == Some("Return date")).is_empty());

    // An invalid start date: an error, and Book is disabled.
    for bad in ["31.04.2027", "29.02.2027", "1.3.2027", "01-03-2027", "aa.bb.cccc", ""] {
        h.type_text(start, bad);
        assert_eq!(h.error(start), "Use the form DD.MM.YYYY", "{bad:?}");
        assert!(h.disabled("Book"), "{bad:?}");
    }
    h.type_text(start, "29.02.2028"); // a leap day
    assert_eq!(h.error(start), "");
    assert!(!h.disabled("Book"));

    // A return flight shows the return date field.
    let kind = h.labeled(ControlKind::Picker, "Flight");
    h.fire(kind, event::CHANGE, "return flight".into());
    let back = h.field("Return date");
    h.type_text(back, "28.02.2028");
    assert_eq!(h.error(back), "The return date is before the start date");
    assert!(h.disabled("Book"));
    h.type_text(back, "29.02.2028"); // the same day is allowed
    assert_eq!(h.error(back), "");
    assert!(!h.disabled("Book"));
    h.type_text(back, "01.03.2028");
    assert!(!h.disabled("Book"));
    h.type_text(back, "1.3.2028");
    assert!(h.disabled("Book"));
    h.type_text(back, "15.01.2029");

    let dialog = h.one(ControlKind::Dialog, |_| true);
    assert!(!h.node(dialog).bool_prop(prop::VALUE));
    h.press("Book");
    assert!(h.node(dialog).bool_prop(prop::VALUE));
    assert_eq!(
        h.node(dialog).str_prop(prop::MESSAGE),
        Some("You booked a return flight on 29.02.2028, back on 15.01.2029.")
    );
    let ok = h.one(ControlKind::Action, |n| n.str_prop(prop::LABEL) == Some("OK"));
    h.fire(ok, event::PRESS, Value::Null);
    assert!(!h.node(dialog).bool_prop(prop::VALUE));

    // Back to one-way: the return date does not matter any more.
    h.type_text(back, "01.01.2000");
    assert!(h.disabled("Book"));
    h.fire(kind, event::CHANGE, "one-way flight".into());
    assert!(h.find(ControlKind::TextField, |n| n.str_prop(prop::LABEL) == Some("Return date")).is_empty());
    assert!(!h.disabled("Book"));
    h.press("Book");
    assert_eq!(h.node(dialog).str_prop(prop::MESSAGE), Some("You booked a one-way flight on 29.02.2028."));
}

#[test]
fn timer_elapses_to_the_duration_and_resets() {
    let art = build("timer");
    let mut h = Harness::start(&art.component);
    let progress = h.one(ControlKind::Progress, |_| true);
    let slider = h.labeled(ControlKind::Slider, "Duration");
    let fraction = |h: &Harness| h.node(progress).num_prop(prop::VALUE).unwrap_or(-1.0);
    assert_eq!(fraction(&h), 0.0);
    assert!(h.texts().contains(&"0.0 s".to_string()));
    assert!(h.texts().contains(&"Duration: 15 s".to_string()));

    // A short duration, so the test needs few ticks.
    h.fire(slider, event::CHANGE, Value::Number(1.0));
    assert!(h.texts().contains(&"Duration: 1 s".to_string()));
    let mut t = Instant::now();
    for _ in 0..5 {
        t += Duration::from_millis(100);
        h.fire_timers(t);
    }
    assert!(h.texts().contains(&"0.5 s".to_string()), "{:?}", h.texts());
    assert!((fraction(&h) - 0.5).abs() < 1e-9);

    // The UI stays responsive while the timer runs: an event between ticks
    // is handled at once.
    h.fire(slider, event::CHANGE, Value::Number(2.0));
    assert!((fraction(&h) - 0.25).abs() < 1e-9);
    h.fire(slider, event::CHANGE, Value::Number(1.0));

    // The elapsed time stops at the duration.
    for _ in 0..10 {
        t += Duration::from_millis(100);
        h.fire_timers(t);
    }
    assert!(h.texts().contains(&"1.0 s".to_string()), "{:?}", h.texts());
    assert_eq!(fraction(&h), 1.0);

    // A longer duration lets the timer go on.
    h.fire(slider, event::CHANGE, Value::Number(2.0));
    assert_eq!(fraction(&h), 0.5);
    for _ in 0..3 {
        t += Duration::from_millis(100);
        h.fire_timers(t);
    }
    assert!(h.texts().contains(&"1.3 s".to_string()), "{:?}", h.texts());

    // Reset starts again from zero.
    h.press("Reset");
    assert!(h.texts().contains(&"0.0 s".to_string()));
    assert_eq!(fraction(&h), 0.0);
    t += Duration::from_millis(100);
    h.fire_timers(t);
    assert!(h.texts().contains(&"0.1 s".to_string()), "{:?}", h.texts());
}
