//! Regression test for the region-content-swap bug (SPEC.md §7.2/§8.4): a
//! reactive region (expression JSX child) that flips between two distinct
//! real elements must never make the host reject a commit with
//! "create: id N is in use".

use plinth_compiler::driver::MemFs;
use plinth_protocol::{ControlKind, Event, NodeId, Value, Writer, event, prop};
use plinth_runner_wasmtime::{Guest, Limits, Runner};
use plinth_ui::tree::{Node, Tree};

fn build(src: &str) -> plinth_compiler::Artifact {
    let fs = MemFs::default().with("app/main.tsx", src);
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[]).expect("compile");
    let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
    artifact.unwrap_or_else(|| panic!("regions test app has errors:\n{}", diags.join("\n")))
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
        let commits = guest.init(&[]).unwrap();
        for commit in commits {
            let errors = tree.apply(&commit).unwrap();
            assert!(errors.is_empty(), "op errors on init: {errors:?}");
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
}

const APP: &str = "\
import { app, signal, Screen, Text, Group, Button } from \"plinth:ui\";

function Section({ title }: { title: string }) {
  return (
    <Group>
      <Text>{title}</Text>
    </Group>
  );
}

function OtherThing({ title }: { title: string }) {
  return <Text>{title}</Text>;
}

function Home() {
  const cond = signal(true);
  return (
    <Screen title=\"Home\">
      <Button label=\"Toggle\" onPress={() => cond.set(!cond())} />
      {cond() ? <Section title=\"A\" /> : <OtherThing title=\"B\" />}
    </Screen>
  );
}

export default app({ screens: { home: { title: \"Home\", component: Home } } });
";

/// A region toggling between two different real elements (not a
/// null<->element transition) repeatedly must never produce a duplicate
/// node id / op error from the host.
#[test]
fn region_toggle_between_two_elements() {
    let art = build(APP);
    let mut h = Harness::start(&art.component);
    let toggle = h.label("Toggle");

    for _ in 0..8 {
        h.fire(toggle, event::PRESS, Value::Null);
    }
}

const APP_NESTED_LISTS: &str = "\
import { app, signal, Screen, Text, Group, Row, Button, List } from \"plinth:ui\";

function ListA() {
  const items = signal([1, 2, 3]);
  return (
    <Group>
      <List items={items()} key={(n) => n} row={(n) => <Row title={\"a\" + n} />} />
    </Group>
  );
}

function ListB() {
  const items = signal([10, 20, 30, 40]);
  return (
    <Group>
      <List items={items()} key={(n) => n} row={(n) => <Row title={\"b\" + n} />} />
    </Group>
  );
}

function Home() {
  const cond = signal(true);
  return (
    <Screen title=\"Home\">
      <Button label=\"Toggle\" onPress={() => cond.set(!cond())} />
      {cond() ? <ListA /> : <ListB />}
    </Screen>
  );
}

export default app({ screens: { home: { title: \"Home\", component: Home } } });
";

/// A region that switches between two components, each containing a
/// nested keyed list, toggled several times.
#[test]
fn region_toggle_between_components_with_nested_lists() {
    let art = build(APP_NESTED_LISTS);
    let mut h = Harness::start(&art.component);
    let toggle = h.label("Toggle");

    for _ in 0..8 {
        h.fire(toggle, event::PRESS, Value::Null);
    }

    let rows = h.find(ControlKind::Row, |_| true);
    assert!(!rows.is_empty(), "expected at least one Row node after toggling");
}
