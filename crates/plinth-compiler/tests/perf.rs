//! Benchmark for large `List`s (SPEC.md Â§7.3, Q6). Not run by default
//! (`#[ignore]`): run with `cargo test -p plinth-compiler --test perf --
//! --ignored --nocapture -- --test-threads=1`. It builds an app with
//! 10,000 rows (each a `Row` with a title, a subtitle and a `Toggle`) and
//! times the guest+tree path (no gpui) for `init`, a single-row update, a
//! filter that drops half the rows, a reverse, and an append of 100 rows.
//! It also prints the size of each commit buffer.

use plinth_compiler::driver::MemFs;
use plinth_protocol::{ControlKind, Event, NodeId, Value, Writer, event, prop};
use plinth_runner_wasmtime::{Guest, Limits, Runner};
use plinth_ui::tree::{Node, Tree};
use std::time::{Duration, Instant};

const APP: &str = r#"
import { app, signal, computed, Screen, Section, List, Row, Toggle, Button, Empty } from "plinth:ui";

export interface Item {
  id: number;
  title: string;
  subtitle: string;
  on: boolean;
}

function makeItems(n: number, offset: number): Item[] {
  const out: Item[] = [];
  let i = 0;
  while (i < n) {
    const id = i + offset;
    out.push({ id, title: `Row ${id}`, subtitle: `Subtitle ${id}`, on: false });
    i = i + 1;
  }
  return out;
}

export const items = signal<Item[]>(makeItems(10000, 0));
export const filterOn = signal(false);

export const visible = computed(() => (filterOn() ? items().filter((it) => it.id % 2 === 0) : items()));

function toggleOne(id: number): void {
  items.set(items().map((it) => (it.id === id ? { ...it, on: !it.on } : it)));
}

function toggleFilter(): void {
  filterOn.set(!filterOn());
}

function reverseAll(): void {
  items.set(items().slice().reverse());
}

function appendMany(): void {
  items.set([...items(), ...makeItems(100, 1000000)]);
}

function App() {
  return (
    <Screen title="Big list">
      <Section title="Actions">
        <Button label="Toggle filter" onPress={toggleFilter} />
        <Button label="Reverse" onPress={reverseAll} />
        <Button label="Append 100" onPress={appendMany} />
      </Section>
      <Section title="Items">
        <List
          items={visible()}
          key={(it) => it.id}
          row={(it) => (
            <Row title={it.title} subtitle={it.subtitle}>
              <Toggle label="On" value={it.on} onChange={() => toggleOne(it.id)} />
            </Row>
          )}
          empty={<Empty title="No items" />}
        />
      </Section>
    </Screen>
  );
}

export default app({
  accent: "teal",
  screens: {
    big: { title: "Big list", icon: "number", component: App },
  },
});
"#;

fn build() -> plinth_compiler::Artifact {
    let fs = MemFs::default().with("app/main.tsx", APP);
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[]).expect("compile");
    let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
    artifact.unwrap_or_else(|| panic!("perf app has errors:\n{}", diags.join("\n")))
}

struct Harness {
    guest: Guest,
    tree: Tree,
    _runner: Runner,
    last_commit_bytes: usize,
}

impl Harness {
    fn start(bytes: &[u8]) -> Self {
        let runner = Runner::new().unwrap();
        let mut guest = runner.load(bytes, Limits::default()).unwrap();
        let mut tree = Tree::new();
        let commits = guest.init(&[]).unwrap();
        for log in guest.take_logs() {
            eprintln!("guest: {log}");
        }
        let mut total = 0;
        for commit in &commits {
            total += commit.len();
            let errors = tree.apply(commit).unwrap();
            assert!(errors.is_empty(), "op errors: {errors:?}");
        }
        Self { guest, tree, _runner: runner, last_commit_bytes: total }
    }

    fn press(&mut self, node: NodeId) -> Duration {
        self.fire(node, event::PRESS, Value::Null)
    }

    fn fire(&mut self, node: NodeId, ev: u16, value: Value) -> Duration {
        let handler = self.tree.get(node).and_then(|n| n.handler(ev)).expect("node has a handler");
        let mut w = Writer::new();
        w.event(&Event::Ui { handler, event: ev, value });
        let start = Instant::now();
        let commits = self.guest.on_event(w.as_bytes()).unwrap();
        let elapsed = start.elapsed();
        for log in self.guest.take_logs() {
            eprintln!("guest: {log}");
        }
        let mut total = 0;
        for commit in &commits {
            total += commit.len();
            let errors = self.tree.apply(commit).unwrap();
            assert!(errors.is_empty(), "op errors: {errors:?}");
        }
        self.last_commit_bytes = total;
        elapsed
    }

    fn list_node(&self) -> NodeId {
        find_one(&self.tree, ControlKind::List)
    }

    fn list_len(&self) -> usize {
        self.tree.get(self.list_node()).unwrap().children.len()
    }

    fn label(&self, label: &str) -> NodeId {
        find_all(&self.tree, ControlKind::Button, |n| n.str_prop(prop::LABEL) == Some(label))[0]
    }

    fn first_toggle(&self) -> NodeId {
        let list = self.list_node();
        let row = self.tree.get(list).unwrap().children[0];
        self.tree.get(row).unwrap().children.iter().copied().find(|c| self.tree.get(*c).unwrap().kind == Some(ControlKind::Toggle)).unwrap()
    }
}

fn find_all(tree: &Tree, kind: ControlKind, pred: impl Fn(&Node) -> bool) -> Vec<NodeId> {
    let mut found = Vec::new();
    let mut stack: Vec<NodeId> = tree.screens().map(|(_, id)| id).collect();
    while let Some(id) = stack.pop() {
        let node = tree.get(id).unwrap();
        if node.kind == Some(kind) && pred(node) {
            found.push(id);
        }
        stack.extend(node.children.iter());
    }
    found
}

fn find_one(tree: &Tree, kind: ControlKind) -> NodeId {
    find_all(tree, kind, |_| true)[0]
}

/// Prints the time (and commit size) of each List operation at 10,000
/// rows. `#[ignore]`: run explicitly, see the module doc. Not a
/// correctness test beyond basic row-count sanity; its job is to print
/// numbers for before/after comparison when fixing a hot spot.
#[test]
#[ignore]
fn list_10k_perf() {
    let artifact = build();

    let start = Instant::now();
    let mut h = Harness::start(&artifact.component);
    let init_time = start.elapsed();
    let init_bytes = h.last_commit_bytes;
    assert_eq!(h.list_len(), 10000);
    println!("init (10,000 rows):      {init_time:?}  ({init_bytes} bytes)");

    let toggle_node = h.first_toggle();
    let t = h.fire(toggle_node, event::CHANGE, Value::Bool(true));
    println!("toggle one row:          {t:?}  ({} bytes)", h.last_commit_bytes);

    let filter_btn = h.label("Toggle filter");
    let t = h.press(filter_btn);
    println!("filter (drop half):      {t:?}  ({} bytes, {} rows left)", h.last_commit_bytes, h.list_len());
    assert_eq!(h.list_len(), 5000);

    // Remove the filter so the reverse below operates on all 10,000 rows.
    let t = h.press(filter_btn);
    println!("un-filter:               {t:?}  ({} bytes)", h.last_commit_bytes);
    assert_eq!(h.list_len(), 10000);

    let reverse_btn = h.label("Reverse");
    let t = h.press(reverse_btn);
    println!("reverse all:             {t:?}  ({} bytes)", h.last_commit_bytes);
    assert_eq!(h.list_len(), 10000);

    let append_btn = h.label("Append 100");
    let t = h.press(append_btn);
    println!("append 100:              {t:?}  ({} bytes)", h.last_commit_bytes);
    assert_eq!(h.list_len(), 10100);
}
