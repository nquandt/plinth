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

// -- wasmtime against wasmi (docs/GAPS.md "wasmi engine", PRODUCTION R4) ------
//
// `engines_speed` runs each workload on both engines in one process, in
// turns (wasmtime, wasmi, wasmtime, ...), so that load from other programs
// affects both engines the same (HANDOFF.md §7). It also prints the fuel
// that each wasmi call used, and the fuel per millisecond, for the
// calibration of the wasmi time limit (`FUEL_PER_MS` in
// `plinth-runner-wasmtime`). Run it in a release build:
//
//   cargo test --release -p plinth-compiler --test perf -- --ignored --nocapture --test-threads=1 engines_speed

#[derive(Clone, Copy, PartialEq)]
enum Engine {
    Wasmtime,
    Wasmi,
}

const ENGINES: [Engine; 2] = [Engine::Wasmtime, Engine::Wasmi];

/// A runner for `engine`. `Runner::new` reads `PLINTH_ENGINE` one time, so
/// the bench sets the variable just before it makes the runner. The bench
/// runs on one thread (`--test-threads=1`).
fn runner_for(engine: Engine) -> Runner {
    // SAFETY: no other thread reads the environment at this time.
    unsafe {
        match engine {
            Engine::Wasmi => std::env::set_var("PLINTH_ENGINE", "wasmi"),
            Engine::Wasmtime => std::env::remove_var("PLINTH_ENGINE"),
        }
    }
    let runner = Runner::new().unwrap();
    unsafe { std::env::remove_var("PLINTH_ENGINE") };
    runner
}

/// The limits of the bench: a long time limit, so that no workload stops
/// (the bench measures the fuel; it does not test the limit).
fn bench_limits() -> Limits {
    Limits { call_timeout: Duration::from_secs(120), ..Limits::default() }
}

/// One guest call: its time, and the fuel it used (wasmi only).
#[derive(Clone, Copy)]
struct Sample {
    time: Duration,
    fuel: Option<u64>,
}

struct Run {
    guest: Guest,
    tree: Tree,
    _runner: Runner,
}

impl Run {
    /// Loads `bytes` on `engine` and calls `init`. Returns the run and the
    /// sample of `init` (load not included).
    fn start(engine: Engine, bytes: &[u8]) -> (Self, Sample) {
        let runner = runner_for(engine);
        let mut guest = runner.load(bytes, bench_limits()).unwrap();
        let seed = plinth_protocol::init_arg::one(plinth_protocol::init_arg::RANDOM_SEED, &1u64.to_le_bytes());
        let t = Instant::now();
        let commits = guest.init(&seed).unwrap();
        let sample = Sample { time: t.elapsed(), fuel: guest.last_call_fuel() };
        let mut run = Self { guest, tree: Tree::new(), _runner: runner };
        run.apply(commits);
        (run, sample)
    }

    fn apply(&mut self, commits: Vec<Vec<u8>>) {
        for log in self.guest.take_logs() {
            eprintln!("guest: {log}");
        }
        for commit in commits {
            let errors = self.tree.apply(&commit).unwrap();
            assert!(errors.is_empty(), "op errors: {errors:?}");
        }
        let errors = self.guest.take_errors();
        assert!(errors.is_empty(), "the app reported errors: {errors:?}");
    }

    fn timed(&mut self, f: impl FnOnce(&mut Guest) -> anyhow::Result<Vec<Vec<u8>>>) -> Sample {
        let t = Instant::now();
        let commits = f(&mut self.guest).unwrap();
        let sample = Sample { time: t.elapsed(), fuel: self.guest.last_call_fuel() };
        self.apply(commits);
        sample
    }

    fn fire(&mut self, node: NodeId, ev: u16, value: Value) -> Sample {
        let handler = self.tree.get(node).and_then(|n| n.handler(ev)).expect("node has a handler");
        let mut w = Writer::new();
        w.event(&Event::Ui { handler, event: ev, value });
        self.timed(|g| g.on_event(w.as_bytes()))
    }

    /// The node with the label `label` that takes a press.
    fn pressable(&self, label: &str) -> NodeId {
        let found = find_all(&self.tree, ControlKind::Button, |n| n.str_prop(prop::LABEL) == Some(label))
            .into_iter()
            .chain(find_all(&self.tree, ControlKind::Pressable, |n| n.str_prop(prop::LABEL) == Some(label)));
        found.into_iter().next().unwrap_or_else(|| panic!("nothing labelled {label}"))
    }

    fn press(&mut self, label: &str) -> Sample {
        let node = self.pressable(label);
        self.fire(node, event::PRESS, Value::Null)
    }

    fn text(&self) -> String {
        let id = find_one(&self.tree, ControlKind::Text);
        self.tree.get(id).unwrap().text.clone().unwrap_or_default()
    }
}

fn build_dir(dir: &str) -> plinth_compiler::Artifact {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(dir);
    let fs = plinth_compiler::driver::DiskFs { root };
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[]).expect("compile");
    let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
    artifact.unwrap_or_else(|| panic!("{dir} has errors:\n{}", diags.join("\n")))
}

/// Workloads of different kinds, each behind one button, for the fuel
/// calibration: a tight loop, strings, allocations, calls, a sort, a map,
/// and a collection with a large live heap.
const WORKLOADS: &str = r#"
import { app, signal, Screen, Text, Button } from "plinth:ui";

interface P {
  x: number;
  y: number;
  tags: string[];
}

const out = signal("");
const live: P[] = [];
// The size of the workloads: 1 in a release build, smaller in a debug
// build (the bench replaces the value).
const K = 1;

function loopWork(): void {
  let s = 0;
  let i = 0;
  while (i < 2000000 / K) {
    s = s + (i % 7);
    i = i + 1;
  }
  out.set("loop " + s);
}

function strings(): void {
  const parts: string[] = [];
  for (let i = 0; i < 20000 / K; i++) {
    const t = `item ${i} of the list`.toUpperCase();
    if (t.indexOf("9") >= 0) {
      parts.push(t.slice(0, 8));
    }
  }
  out.set("strings " + parts.join(",").length);
}

function allocs(): void {
  const a: P[] = [];
  for (let i = 0; i < 30000 / K; i++) {
    a.push({ x: i, y: i * 2, tags: ["a", "b"] });
  }
  out.set("allocs " + a.length);
}

function fib(n: number): number {
  if (n < 2) {
    return n;
  }
  return fib(n - 1) + fib(n - 2);
}

function calls(): void {
  out.set("calls " + fib(K > 1 ? 17 : 22));
}

function sortWork(): void {
  const a: number[] = [];
  for (let i = 0; i < 10000 / K; i++) {
    a.push((i * 7919) % 10000);
  }
  a.sort((x, y) => x - y);
  out.set("sort " + a[5]);
}

function maps(): void {
  const m = new Map<string, number>();
  for (let i = 0; i < 10000 / K; i++) {
    m.set("k" + i, i);
  }
  let s = 0;
  for (let i = 0; i < 10000 / K; i++) {
    if (m.has("k" + i)) {
      s = s + 1;
    }
  }
  out.set("maps " + s);
}

function gcSetup(): void {
  for (let i = 0; i < 50000 / K; i++) {
    live.push({ x: i, y: i, tags: [] });
  }
  out.set("live " + live.length);
}

function garbage(): void {
  let n = 0;
  for (let i = 0; i < 20000 / K; i++) {
    const p: P = { x: i, y: i, tags: ["g"] };
    n = n + p.tags.length;
  }
  out.set("garbage " + n);
}

function Home() {
  return (
    <Screen title="Work">
      <Text>{out()}</Text>
      <Button label="loop" onPress={loopWork} />
      <Button label="strings" onPress={strings} />
      <Button label="allocs" onPress={allocs} />
      <Button label="calls" onPress={calls} />
      <Button label="sort" onPress={sortWork} />
      <Button label="maps" onPress={maps} />
      <Button label="gc setup" onPress={gcSetup} />
      <Button label="garbage" onPress={garbage} />
    </Screen>
  );
}

export default app({ screens: { home: { title: "Work", component: Home } } });
"#;

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

/// The samples of one workload on both engines.
#[derive(Default)]
struct Row {
    wasmtime: Vec<Duration>,
    wasmi: Vec<Duration>,
    fuel: Vec<u64>,
}

impl Row {
    fn add(&mut self, engine: Engine, s: Sample) {
        match engine {
            Engine::Wasmtime => self.wasmtime.push(s.time),
            Engine::Wasmi => {
                self.wasmi.push(s.time);
                self.fuel.push(s.fuel.expect("wasmi gives the fuel"));
            }
        }
    }

    fn print(&self, name: &str) {
        let wt = median(self.wasmtime.clone());
        let wi = median(self.wasmi.clone());
        let mut fuel = self.fuel.clone();
        fuel.sort();
        let fuel = fuel[fuel.len() / 2];
        let ms = wi.as_secs_f64() * 1000.0;
        println!(
            "| {name} | {:.2} ms | {:.2} ms | {:.1}x | {fuel} | {:.0} |",
            wt.as_secs_f64() * 1000.0,
            ms,
            wi.as_secs_f64() / wt.as_secs_f64().max(1e-9),
            fuel as f64 / ms.max(1e-6)
        );
    }
}

#[test]
#[ignore]
fn engines_speed() {
    // A debug build of wasmi is about 100 times slower: fewer rounds.
    let rounds: usize = if cfg!(debug_assertions) { 1 } else { 5 };
    println!("| Workload | wasmtime | wasmi | wasmi / wasmtime | wasmi fuel | fuel per ms |");
    println!("|---|---|---|---|---|---|");

    // The 10,000-row List: `init` (a new guest for each round).
    let list = build();
    let mut row = Row::default();
    for _ in 0..rounds {
        for engine in ENGINES {
            let (run, s) = Run::start(engine, &list.component);
            assert_eq!(run.tree.get(find_one(&run.tree, ControlKind::List)).unwrap().children.len(), 10000);
            row.add(engine, s);
        }
    }
    row.print("List, init of 10,000 rows");

    // Pong: one frame of the game loop, the two engines in turns.
    let pong = build_dir("pong");
    let mut runs: Vec<Run> = ENGINES.iter().map(|e| Run::start(*e, &pong.component).0).collect();
    for r in &mut runs {
        r.press("Start");
        assert!(r.guest.wants_frames());
    }
    let mut row = Row::default();
    let mut now = Instant::now();
    for _ in 0..300 {
        now += Duration::from_nanos(16_666_667);
        for (i, engine) in ENGINES.iter().enumerate() {
            let s = runs[i].timed(|g| g.fire_frame(now));
            row.add(*engine, s);
        }
    }
    row.print("Pong, one frame");

    // 7GUIs Cells: one edit (select B2, type, Set), a new guest each round.
    let cells = build_dir("7guis/cells");
    let mut row = Row::default();
    for round in 0..rounds {
        for engine in ENGINES {
            let (mut run, _) = Run::start(engine, &cells.component);
            run.press("B2");
            let field = find_one(&run.tree, ControlKind::TextField);
            let text = format!("{}", 10 + round);
            run.tree.set_local_prop(field, prop::VALUE, Value::Str(text.clone()));
            let a = run.fire(field, event::CHANGE, Value::Str(text));
            let b = run.press("Set");
            let fuel = match (a.fuel, b.fuel) {
                (Some(x), Some(y)) => Some(x + y),
                _ => None,
            };
            row.add(engine, Sample { time: a.time + b.time, fuel });
        }
    }
    row.print("Cells, one edit (type and Set)");

    // The fuel calibration workloads.
    let work = {
        // A debug build of wasmi is about 100 times slower: smaller workloads.
        let k = if cfg!(debug_assertions) { "const K = 20;" } else { "const K = 1;" };
        let fs = MemFs::default().with("app/main.tsx", &WORKLOADS.replace("const K = 1;", k));
        let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[]).expect("compile");
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        artifact.unwrap_or_else(|| panic!("the workloads have errors:\n{}", diags.join("\n")))
    };
    let mut runs: Vec<Run> = ENGINES.iter().map(|e| Run::start(*e, &work.component).0).collect();
    for name in ["loop", "strings", "allocs", "calls", "sort", "maps"] {
        let mut row = Row::default();
        for _ in 0..rounds {
            for (i, engine) in ENGINES.iter().enumerate() {
                let s = runs[i].press(name);
                assert!(runs[i].text().starts_with(name), "{name}: {}", runs[i].text());
                row.add(*engine, s);
            }
        }
        row.print(name);
    }
    for r in &mut runs {
        r.press("gc setup");
    }
    let mut row = Row::default();
    for _ in 0..rounds {
        for (i, engine) in ENGINES.iter().enumerate() {
            row.add(*engine, runs[i].press("garbage"));
        }
    }
    row.print("garbage and a collection (50,000 live objects)");
    println!("(times: the guest call only; medians of {rounds} rounds, Pong of 300 frames)");
}
