//! Randomized correctness check for the `List` keyed-diff reconciler and
//! the host's batched `Insert`/`Move` apply (SPEC.md §7.3, §8.3): many
//! random moves/inserts/removes of a keyed list, checking after every
//! update that the host tree's row order matches an independently kept
//! expected order, with no op errors. Runs once in plain mode and once
//! under GC stress (`plinth_protocol::init_arg::GC_STRESS`, SPEC.md §16).

use plinth_compiler::driver::MemFs;
use plinth_protocol::{ControlKind, Event, NodeId, Value, Writer, event, prop};
use plinth_runner_wasmtime::{Guest, Limits, Runner};
use plinth_ui::tree::{Node, Tree};

const APP: &str = r#"
import { app, signal, Screen, Section, List, Row, Toggle, NumberField, Button, Empty } from "plinth:ui";

export interface Item {
  id: number;
  title: string;
  subtitle: string;
  on: boolean;
}

function makeItem(id: number): Item {
  return { id, title: `Row ${id}`, subtitle: `Subtitle ${id}`, on: false };
}

function makeItems(n: number): Item[] {
  const out: Item[] = [];
  let i = 0;
  while (i < n) {
    out.push(makeItem(i));
    i = i + 1;
  }
  return out;
}

export const items = signal<Item[]>(makeItems(20));
export const opKind = signal(0);
export const opA = signal(0);
export const opB = signal(0);
export const nextId = signal(1000);

function moveItem(arr: Item[], a: number, b: number): Item[] {
  if (a < 0 || a >= arr.length || b < 0 || b >= arr.length || a === b) return arr;
  const it = arr[a];
  const without: Item[] = [];
  let i = 0;
  while (i < arr.length) {
    if (i !== a) without.push(arr[i]);
    i = i + 1;
  }
  const insertAt = a < b ? b - 1 : b;
  const out: Item[] = [];
  let k = 0;
  while (k < without.length) {
    if (k === insertAt) out.push(it);
    out.push(without[k]);
    k = k + 1;
  }
  if (insertAt >= without.length) out.push(it);
  return out;
}

function insertItem(arr: Item[], at: number, id: number): Item[] {
  const clamped = at < 0 ? 0 : (at > arr.length ? arr.length : at);
  const out: Item[] = [];
  let i = 0;
  while (i < arr.length) {
    if (i === clamped) out.push(makeItem(id));
    out.push(arr[i]);
    i = i + 1;
  }
  if (clamped >= arr.length) out.push(makeItem(id));
  return out;
}

function removeItem(arr: Item[], at: number): Item[] {
  if (at < 0 || at >= arr.length) return arr;
  const out: Item[] = [];
  let i = 0;
  while (i < arr.length) {
    if (i !== at) out.push(arr[i]);
    i = i + 1;
  }
  return out;
}

function apply(): void {
  const kind = opKind();
  const a = opA();
  const b = opB();
  if (kind === 0) {
    items.set(moveItem(items(), a, b));
  } else if (kind === 1) {
    items.set(insertItem(items(), a, nextId()));
    nextId.set(nextId() + 1);
  } else {
    items.set(removeItem(items(), a));
  }
}

function App() {
  return (
    <Screen title="List diff">
      <Section title="Controls">
        <NumberField label="Kind" value={opKind} min={0} max={2} step={1} />
        <NumberField label="A" value={opA} min={0} max={100000} step={1} />
        <NumberField label="B" value={opB} min={0} max={100000} step={1} />
        <Button label="Apply" onPress={apply} />
      </Section>
      <Section title="Items">
        <List
          items={items()}
          key={(it) => it.id}
          row={(it) => (
            <Row title={it.title} subtitle={it.subtitle}>
              <Toggle label="On" value={it.on} onChange={() => {}} />
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
    diff: { title: "List diff", icon: "number", component: App },
  },
});
"#;

fn build() -> plinth_compiler::Artifact {
    let fs = MemFs::default().with("app/main.tsx", APP);
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[]).expect("compile");
    let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
    artifact.unwrap_or_else(|| panic!("list_diff app has errors:\n{}", diags.join("\n")))
}

/// A tiny fixed-seed PRNG (xorshift32), so the test is deterministic and
/// needs no new dependency.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// A number in `0..bound` (bound must be > 0).
    fn below(&mut self, bound: usize) -> usize {
        (self.next() as usize) % bound
    }
}

struct Harness {
    guest: Guest,
    tree: Tree,
    _runner: Runner,
}

impl Harness {
    fn start(bytes: &[u8], gc_stress: bool) -> Self {
        let runner = Runner::new().unwrap();
        let mut guest = runner.load(bytes, Limits::default()).unwrap();
        let mut tree = Tree::new();
        let args = if gc_stress { plinth_protocol::init_arg::one(plinth_protocol::init_arg::GC_STRESS, &[]) } else { Vec::new() };
        let commits = guest.init(&args).unwrap();
        for log in guest.take_logs() {
            eprintln!("guest: {log}");
        }
        for commit in commits {
            let errors = tree.apply(&commit).unwrap();
            assert!(errors.is_empty(), "init op errors: {errors:?}");
        }
        Self { guest, tree, _runner: runner }
    }

    fn set_number(&mut self, label: &str, value: f64) {
        let field = self.one(ControlKind::NumberField, |n| n.str_prop(prop::LABEL) == Some(label));
        self.tree.set_local_prop(field, prop::VALUE, Value::Number(value));
        self.fire(field, event::CHANGE, Value::Number(value));
    }

    fn press(&mut self, label: &str) {
        let btn = self.one(ControlKind::Button, |n| n.str_prop(prop::LABEL) == Some(label));
        self.fire(btn, event::PRESS, Value::Null);
    }

    fn fire(&mut self, node: NodeId, ev: u16, value: Value) {
        let handler = self.tree.get(node).and_then(|n| n.handler(ev)).expect("node has a handler");
        let mut w = Writer::new();
        w.event(&Event::Ui { handler, event: ev, value });
        let commits = self.guest.on_event(w.as_bytes()).unwrap();
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

    /// The `Row` titles in host child order, e.g. `["Row 3", "Row 1", ...]`.
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

/// Applies `n` random moves/inserts/removes to `model` (the independently
/// kept expected id order) and the guest+host, asserting after each one
/// that the host's row order (by title, which encodes the id) matches.
fn run_random_ops(h: &mut Harness, model: &mut Vec<u32>, mut next_id: u32, n: usize, seed: u32) {
    let mut rng = Rng(seed);
    for step in 0..n {
        if model.is_empty() {
            // Nothing to move or remove; always insert.
            let id = next_id;
            next_id += 1;
            let at = 0;
            model.insert(at, id);
            h.set_number("Kind", 1.0);
            h.set_number("A", at as f64);
            h.press("Apply");
        } else {
            match rng.below(3) {
                0 => {
                    let a = rng.below(model.len());
                    let b = rng.below(model.len());
                    let id = model.remove(a);
                    let b_after_remove = if b > a { b - 1 } else { b };
                    model.insert(b_after_remove.min(model.len()), id);
                    h.set_number("Kind", 0.0);
                    h.set_number("A", a as f64);
                    h.set_number("B", b as f64);
                    h.press("Apply");
                }
                1 => {
                    let at = rng.below(model.len() + 1);
                    let id = next_id;
                    next_id += 1;
                    model.insert(at, id);
                    h.set_number("Kind", 1.0);
                    h.set_number("A", at as f64);
                    h.press("Apply");
                }
                _ => {
                    let at = rng.below(model.len());
                    model.remove(at);
                    h.set_number("Kind", 2.0);
                    h.set_number("A", at as f64);
                    h.press("Apply");
                }
            }
        }
        let expected: Vec<String> = model.iter().map(|id| format!("Row {id}")).collect();
        assert_eq!(h.row_titles(), expected, "mismatch after step {step} (seed {seed})");
    }
}

fn initial_model() -> Vec<u32> {
    (0..20).collect()
}

#[test]
fn random_reorders_inserts_and_removes_match_the_model() {
    let art = build();
    let mut h = Harness::start(&art.component, false);
    let mut model = initial_model();
    assert_eq!(h.row_titles(), model.iter().map(|id| format!("Row {id}")).collect::<Vec<_>>());
    run_random_ops(&mut h, &mut model, 1000, 300, 0xC0FF_EE42);
}

#[test]
fn random_reorders_inserts_and_removes_match_the_model_under_gc_stress() {
    let art = build();
    let mut h = Harness::start(&art.component, true);
    let mut model = initial_model();
    assert_eq!(h.row_titles(), model.iter().map(|id| format!("Row {id}")).collect::<Vec<_>>());
    run_random_ops(&mut h, &mut model, 1000, 150, 0x1234_5678);
}
