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
