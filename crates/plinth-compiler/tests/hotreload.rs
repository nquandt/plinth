//! Hot reload keeps signal state across a reload when the shape did not
//! change (SPEC.md §13). This compiles two versions of a small app, pokes
//! state through events, snapshots it, compiles the second version (with
//! one signal's type changed) and checks that the matching signal kept its
//! value while the mismatched one reset.

use plinth_compiler::driver::MemFs;
use plinth_protocol::{ControlKind, Event, NodeId, Op, Value, Writer, decode_ops, event};
use plinth_runner_wasmtime::{Guest, Limits, Runner};
use plinth_ui::tree::{Node, Tree};

const VERSION_A: &str = r#"
import { app, signal, Screen, Section, Heading, Text, Button } from "plinth:ui";

const count = signal(0);
const greeting = signal("hi");

function Main() {
  return (
    <Screen title="Version A">
      <Section title="state">
        <Heading level={1}>{count()}</Heading>
        <Text>{greeting()}</Text>
      </Section>
      <Section title="actions">
        <Button label="Increment" onPress={() => count.set(count() + 1)} />
      </Section>
    </Screen>
  );
}

export default app({
  screens: { main: { title: "Version A", component: Main } },
});
"#;

// Version B: the screen title text changed (checks the reload still shows
// the new UI), `count` keeps the same shape (number, so it is restored),
// and `greeting` changed shape from `string` to `number` (so it resets to
// its new initializer instead of restoring the old string).
const VERSION_B: &str = r#"
import { app, signal, Screen, Section, Heading, Text, Button } from "plinth:ui";

const count = signal(0);
const greeting = signal(99);

function Main() {
  return (
    <Screen title="Version B">
      <Section title="state">
        <Heading level={1}>{count()}</Heading>
        <Text>{greeting()}</Text>
      </Section>
      <Section title="actions">
        <Button label="Increment" onPress={() => count.set(count() + 1)} />
      </Section>
    </Screen>
  );
}

export default app({
  screens: { main: { title: "Version B", component: Main } },
});
"#;

fn build_dev(src: &str) -> plinth_compiler::Artifact {
    let fs = MemFs::default().with("app/main.tsx", src);
    let (front, artifact) = plinth_compiler::compile_ex(&fs, &[], true).expect("compile");
    let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
    artifact.unwrap_or_else(|| panic!("compile errors:\n{}", diags.join("\n")))
}

struct Harness {
    guest: Guest,
    tree: Tree,
    _runner: Runner,
}

impl Harness {
    fn start(bytes: &[u8], args: &[u8]) -> Self {
        let runner = Runner::new().unwrap();
        let mut guest = runner.load(bytes, Limits::default()).unwrap();
        let mut tree = Tree::new();
        let commits = guest.init(args).unwrap();
        for log in guest.take_logs() {
            eprintln!("guest: {log}");
        }
        for commit in commits {
            let errors = tree.apply(&commit).unwrap();
            assert!(errors.is_empty(), "op errors: {errors:?}");
        }
        Self { guest, tree, _runner: runner }
    }

    fn fire(&mut self, node: NodeId, ev: u16, value: Value) {
        let handler = self.tree.get(node).and_then(|n| n.handler(ev)).expect("node has a handler");
        let mut w = Writer::new();
        w.event(&Event::Ui { handler, event: ev, value });
        let commits = self.guest.on_event(w.as_bytes()).unwrap();
        for commit in commits {
            let errors = self.tree.apply(&commit).unwrap();
            assert!(errors.is_empty(), "op errors: {errors:?}");
        }
    }

    /// Dev-only (SPEC.md §13): asks the guest for a hot-reload snapshot and
    /// returns the raw bytes (not run through the `Tree`, which ignores
    /// `Op::Snapshot`; the host reads the op buffer directly).
    fn snapshot(&mut self) -> Vec<u8> {
        let mut w = Writer::new();
        w.event(&Event::SnapshotRequest);
        let commits = self.guest.on_event(w.as_bytes()).unwrap();
        for commit in &commits {
            for op in decode_ops(commit).unwrap() {
                if let Op::Snapshot { bytes } = op {
                    return bytes;
                }
            }
        }
        panic!("the guest did not reply with a snapshot");
    }

    fn one(&self, kind: ControlKind, pred: impl Fn(&Node) -> bool) -> NodeId {
        let mut stack: Vec<NodeId> = self.tree.screens().map(|(_, id)| id).collect();
        let mut found = Vec::new();
        while let Some(id) = stack.pop() {
            let node = self.tree.get(id).unwrap();
            if node.kind == Some(kind) && pred(node) {
                found.push(id);
            }
            stack.extend(node.children.iter().copied());
        }
        assert_eq!(found.len(), 1, "expected exactly one match");
        found[0]
    }

    fn heading_text(&self) -> String {
        let h = self.one(ControlKind::Heading, |_| true);
        self.tree.get(h).unwrap().text.clone().unwrap_or_default()
    }

    fn texts(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack: Vec<NodeId> = self.tree.screens().map(|(_, id)| id).collect();
        while let Some(id) = stack.pop() {
            let node = self.tree.get(id).unwrap();
            if node.kind == Some(ControlKind::Text) {
                out.push(node.text.clone().unwrap_or_default());
            }
            stack.extend(node.children.iter().copied());
        }
        out
    }

    fn screen_title(&self) -> String {
        let root = self.tree.current_root().unwrap();
        root.str_prop(plinth_protocol::prop::TITLE).unwrap_or_default().to_owned()
    }
}

#[test]
fn signals_with_a_matching_shape_survive_a_reload_and_mismatches_reset() {
    let a = build_dev(VERSION_A);
    let mut h = Harness::start(&a.component, &[]);
    assert_eq!(h.screen_title(), "Version A");
    assert_eq!(h.heading_text(), "0");
    assert_eq!(h.texts(), ["hi"]);

    // Change state through events (SPEC.md §13: a real edit, not the
    // initializer), then snapshot before the reload.
    let inc = h.one(ControlKind::Button, |n| n.str_prop(plinth_protocol::prop::LABEL) == Some("Increment"));
    h.fire(inc, event::PRESS, Value::Null);
    h.fire(inc, event::PRESS, Value::Null);
    h.fire(inc, event::PRESS, Value::Null);
    assert_eq!(h.heading_text(), "3");
    let snapshot = h.snapshot();
    assert!(!snapshot.is_empty());

    // Compile version B (changed UI text; `count` keeps its shape,
    // `greeting` changed from `string` to `number`) and start it with the
    // snapshot passed as `init`'s `args`.
    let b = build_dev(VERSION_B);
    let h2 = Harness::start(&b.component, &plinth_protocol::init_arg::one(plinth_protocol::init_arg::SNAPSHOT, &snapshot));
    assert_eq!(h2.screen_title(), "Version B", "the new UI text shows");
    assert_eq!(h2.heading_text(), "3", "count kept its value: same key, same shape");
    assert_eq!(h2.texts(), ["99"], "greeting's shape changed, so it reset to its new initializer, not \"hi\"");
}

// Component-local signals (SPEC.md §13): a signal declared inside a
// component function, not at module scope.
const LOCAL_A: &str = r#"
import { app, signal, Screen, Section, Heading, Button } from "plinth:ui";

function Main() {
  const count = signal(0);
  return (
    <Screen title="Version A">
      <Section title="state">
        <Heading level={1}>{count()}</Heading>
      </Section>
      <Section title="actions">
        <Button label="Increment" onPress={() => count.set(count() + 1)} />
      </Section>
    </Screen>
  );
}

export default app({
  screens: { main: { title: "Version A", component: Main } },
});
"#;

// Same declaration, only the screen title changed.
const LOCAL_B: &str = r#"
import { app, signal, Screen, Section, Heading, Button } from "plinth:ui";

function Main() {
  const count = signal(0);
  return (
    <Screen title="Version B">
      <Section title="state">
        <Heading level={1}>{count()}</Heading>
      </Section>
      <Section title="actions">
        <Button label="Increment" onPress={() => count.set(count() + 1)} />
      </Section>
    </Screen>
  );
}

export default app({
  screens: { main: { title: "Version B", component: Main } },
});
"#;

// Same position, but the local signal's type changed from number to
// string: it must reset, like a module-level signal would.
const LOCAL_C_TYPE_CHANGED: &str = r#"
import { app, signal, Screen, Section, Text, Button } from "plinth:ui";

function Main() {
  const count = signal("0");
  return (
    <Screen title="Version C">
      <Section title="state">
        <Text>{count()}</Text>
      </Section>
      <Section title="actions">
        <Button label="Increment" onPress={() => count.set("x")} />
      </Section>
    </Screen>
  );
}

export default app({
  screens: { main: { title: "Version C", component: Main } },
});
"#;

#[test]
fn a_component_local_signal_survives_a_reload() {
    let a = build_dev(LOCAL_A);
    let mut h = Harness::start(&a.component, &[]);
    assert_eq!(h.heading_text(), "0");

    let inc = h.one(ControlKind::Button, |n| n.str_prop(plinth_protocol::prop::LABEL) == Some("Increment"));
    h.fire(inc, event::PRESS, Value::Null);
    h.fire(inc, event::PRESS, Value::Null);
    h.fire(inc, event::PRESS, Value::Null);
    assert_eq!(h.heading_text(), "3");
    let snapshot = h.snapshot();

    let b = build_dev(LOCAL_B);
    let h2 = Harness::start(&b.component, &plinth_protocol::init_arg::one(plinth_protocol::init_arg::SNAPSHOT, &snapshot));
    assert_eq!(h2.screen_title(), "Version B");
    assert_eq!(h2.heading_text(), "3", "the component-local signal kept its value across the reload");
}

#[test]
fn a_component_local_signal_whose_shape_changed_resets() {
    let a = build_dev(LOCAL_A);
    let mut h = Harness::start(&a.component, &[]);
    let inc = h.one(ControlKind::Button, |n| n.str_prop(plinth_protocol::prop::LABEL) == Some("Increment"));
    h.fire(inc, event::PRESS, Value::Null);
    h.fire(inc, event::PRESS, Value::Null);
    let snapshot = h.snapshot();

    let c = build_dev(LOCAL_C_TYPE_CHANGED);
    let h2 = Harness::start(&c.component, &plinth_protocol::init_arg::one(plinth_protocol::init_arg::SNAPSHOT, &snapshot));
    assert_eq!(h2.texts(), ["0"], "the shape changed (number -> string), so it reset to its new initializer");
}

// List rows (SPEC.md §13): each row of a keyed `List` gets its own
// component-local signal instance, kept separate by the row's key.
const ROWS_A: &str = r#"
import { app, signal, Screen, Section, List, Row } from "plinth:ui";

const ids = signal([1, 2, 3]);

function Main() {
  return (
    <Screen title="Rows A">
      <Section title="rows">
        <List
          items={ids()}
          key={(n) => n}
          row={(n) => {
            const on = signal(false);
            return (
              <Row
                title={`row ${n}`}
                subtitle={on() ? "on" : "off"}
                onPress={() => on.set(!on())}
              />
            );
          }}
        />
      </Section>
    </Screen>
  );
}

export default app({
  screens: { main: { title: "Rows A", component: Main } },
});
"#;

// Same rows, row 2 removed: its toggle state must not resurface on a new
// row, and the surviving rows keep their own state.
const ROWS_B: &str = r#"
import { app, signal, Screen, Section, List, Row } from "plinth:ui";

const ids = signal([1, 3]);

function Main() {
  return (
    <Screen title="Rows B">
      <Section title="rows">
        <List
          items={ids()}
          key={(n) => n}
          row={(n) => {
            const on = signal(false);
            return (
              <Row
                title={`row ${n}`}
                subtitle={on() ? "on" : "off"}
                onPress={() => on.set(!on())}
              />
            );
          }}
        />
      </Section>
    </Screen>
  );
}

export default app({
  screens: { main: { title: "Rows B", component: Main } },
});
"#;

#[test]
fn list_rows_keep_their_own_state_keyed_by_row_key_and_dropped_rows_do_not_leak() {
    let a = build_dev(ROWS_A);
    let mut h = Harness::start(&a.component, &[]);
    let row1 = h.one(ControlKind::Row, |n| n.str_prop(plinth_protocol::prop::TITLE) == Some("row 1"));
    let row2 = h.one(ControlKind::Row, |n| n.str_prop(plinth_protocol::prop::TITLE) == Some("row 2"));
    // Toggle row 1 on, leave row 2 and row 3 off.
    h.fire(row1, event::PRESS, Value::Null);
    assert_eq!(h.tree.get(row1).unwrap().str_prop(plinth_protocol::prop::SUBTITLE), Some("on"));
    assert_eq!(h.tree.get(row2).unwrap().str_prop(plinth_protocol::prop::SUBTITLE), Some("off"));
    let snapshot = h.snapshot();

    // Reload with row 2 removed; rows 1 and 3 keep their own state.
    let b = build_dev(ROWS_B);
    let h2 = Harness::start(&b.component, &plinth_protocol::init_arg::one(plinth_protocol::init_arg::SNAPSHOT, &snapshot));
    let row1b = h2.one(ControlKind::Row, |n| n.str_prop(plinth_protocol::prop::TITLE) == Some("row 1"));
    let row3b = h2.one(ControlKind::Row, |n| n.str_prop(plinth_protocol::prop::TITLE) == Some("row 3"));
    assert_eq!(h2.tree.get(row1b).unwrap().str_prop(plinth_protocol::prop::SUBTITLE), Some("on"), "row 1 kept its state");
    assert_eq!(h2.tree.get(row3b).unwrap().str_prop(plinth_protocol::prop::SUBTITLE), Some("off"), "row 3 was never toggled");
}

#[test]
fn a_release_build_does_not_answer_a_snapshot_request() {
    // SPEC.md §13: only a dev build registers signals and answers
    // `Event::SnapshotRequest`. A release build silently ignores it (the
    // event decodes fine; `on_event`'s `_ => {}` catches it), so no
    // `Op::Snapshot` ever comes back and release artifacts never carry
    // this code.
    let fs = MemFs::default().with("app/main.tsx", VERSION_A);
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[]).expect("compile");
    let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
    let art = artifact.unwrap_or_else(|| panic!("compile errors:\n{}", diags.join("\n")));

    let runner = Runner::new().unwrap();
    let mut guest = runner.load(&art.component, Limits::default()).unwrap();
    guest.init(&[]).unwrap();
    let mut w = Writer::new();
    w.event(&Event::SnapshotRequest);
    let commits = guest.on_event(w.as_bytes()).unwrap();
    for commit in &commits {
        for op in decode_ops(commit).unwrap() {
            assert!(!matches!(op, Op::Snapshot { .. }), "a release build must never emit a snapshot");
        }
    }
}
