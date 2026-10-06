//! `<Grid>` behaves like the keyed reconciler of `<List>` (SPEC.md §6.3,
//! §8.4): children track the `items` signal in order and count across add,
//! remove and reverse operations, with no op errors.

use plinth_compiler::driver::MemFs;
use plinth_protocol::{ControlKind, Event, NodeId, Value, Writer, event, prop};
use plinth_runner_wasmtime::{Guest, Limits, Runner};
use plinth_ui::tree::{Node, Tree};

const APP: &str = "\
import { app, signal, Screen, Grid, Text, Button } from \"plinth:ui\";

type Item = { id: number; name: string };

function Home() {
  const items = signal<Item[]>([
    { id: 1, name: \"One\" },
    { id: 2, name: \"Two\" },
    { id: 3, name: \"Three\" },
  ]);
  let nextId = 4;

  const add = () => {
    items.update((xs) => [...xs, { id: nextId, name: `Item ${nextId}` }]);
    nextId++;
  };
  const remove = () => {
    items.update((xs) => xs.slice(0, -1));
  };
  const reverse = () => {
    items.update((xs) => [...xs].reverse());
  };

  return (
    <Screen title=\"Home\">
      <Button label=\"Add\" onPress={add} />
      <Button label=\"Remove\" onPress={remove} />
      <Button label=\"Reverse\" onPress={reverse} />
      <Grid items={items()} key={(x) => x.id} cell={(x) => <Text>{x.name}</Text>} />
    </Screen>
  );
}

export default app({ screens: { home: { title: \"Home\", component: Home } } });
";

fn build() -> plinth_compiler::Artifact {
    let fs = MemFs::default().with("app/main.tsx", APP);
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[]).expect("compile");
    let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
    artifact.unwrap_or_else(|| panic!("grid test app has errors:\n{}", diags.join("\n")))
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

    fn grid_names(&self) -> Vec<String> {
        let grid = self.one(ControlKind::Grid, |_| true);
        self.tree
            .get(grid)
            .unwrap()
            .children
            .iter()
            .filter_map(|c| self.tree.get(*c))
            .filter_map(|n| n.text.clone())
            .collect()
    }
}

#[test]
fn grid_tracks_add_remove_reverse() {
    let art = build();
    let mut h = Harness::start(&art.component);

    assert_eq!(h.grid_names(), vec!["One", "Two", "Three"]);

    h.fire(h.label("Add"), event::PRESS, Value::Null);
    assert_eq!(h.grid_names(), vec!["One", "Two", "Three", "Item 4"]);

    h.fire(h.label("Reverse"), event::PRESS, Value::Null);
    assert_eq!(h.grid_names(), vec!["Item 4", "Three", "Two", "One"]);

    h.fire(h.label("Remove"), event::PRESS, Value::Null);
    assert_eq!(h.grid_names(), vec!["Item 4", "Three", "Two"]);

    h.fire(h.label("Reverse"), event::PRESS, Value::Null);
    assert_eq!(h.grid_names(), vec!["Two", "Three", "Item 4"]);

    h.fire(h.label("Remove"), event::PRESS, Value::Null);
    h.fire(h.label("Remove"), event::PRESS, Value::Null);
    h.fire(h.label("Remove"), event::PRESS, Value::Null);
    assert_eq!(h.grid_names(), Vec::<String>::new());
}
