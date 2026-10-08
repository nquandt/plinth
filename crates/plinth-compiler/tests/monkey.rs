//! A monkey test: every example app gets thousands of random UI events in
//! the desktop runner (presses, typed text, toggles, slider values, keys,
//! pointer moves, list scrolls), with timers, frames and dialog answers in
//! between, once normally and once in GC stress mode (a collection after
//! every call, freed memory poisoned).
//!
//! It checks what a long session of a real user would show:
//! - the app never traps (an uncaught app error is allowed and counted);
//! - every commit applies to the host tree with no op error;
//! - the guest memory stops growing: the second half of the run, which does
//!   the same kind of work as the first, must not grow the memory by more
//!   than a small margin.
//!
//! The random source is fixed. `PLINTH_MONKEY_STEPS` sets the events per
//! app (default 400), `PLINTH_MONKEY_APP` runs only the apps whose name
//! contains it. A failure prints the app, the step and the last events.

use plinth_compiler::driver::MemFs;
use plinth_protocol::{ControlKind, Event, NodeId, Value, Writer, event, prop};
use plinth_runner_wasmtime::kv::Kv;
use plinth_runner_wasmtime::policy::Policy;
use plinth_runner_wasmtime::{Guest, Limits, MemoryClipboard, Runner};
use plinth_ui::tree::Tree;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
    fn pick<T: Clone>(&mut self, xs: &[T]) -> T {
        xs[self.below(xs.len())].clone()
    }
}

struct Project {
    name: String,
    root: PathBuf,
    files: HashMap<String, String>,
    capabilities: Vec<String>,
}

fn collect(dir: &Path, root: &Path, out: &mut HashMap<String, String>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, root, out);
        } else if p.extension().is_some_and(|x| x == "ts" || x == "tsx") {
            let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            out.insert(rel, std::fs::read_to_string(&p).unwrap());
        }
    }
}

fn projects() -> Vec<Project> {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut roots = Vec::new();
    for e in std::fs::read_dir(&base).unwrap().flatten() {
        let p = e.path();
        if p.join("app/main.tsx").is_file() {
            roots.push(p);
        } else if p.is_dir() {
            roots.extend(std::fs::read_dir(&p).unwrap().flatten().map(|e| e.path()).filter(|p| p.join("app/main.tsx").is_file()));
        }
    }
    roots.sort();
    roots
        .into_iter()
        .map(|root| {
            let mut files = HashMap::new();
            collect(&root.join("app"), &root, &mut files);
            if let Ok(rd) = std::fs::read_dir(root.join("assets")) {
                for e in rd.flatten() {
                    files.insert(format!("assets/{}", e.file_name().to_string_lossy()), String::new());
                }
            }
            let text = std::fs::read_to_string(root.join("plinth.toml")).unwrap_or_default();
            let capabilities = text
                .parse::<toml::Value>()
                .ok()
                .and_then(|v| v.get("capabilities")?.as_array().cloned())
                .map(|caps| caps.iter().filter_map(|c| c.get("name")?.as_str().map(str::to_owned)).collect())
                .unwrap_or_default();
            let name = root.strip_prefix(&base).unwrap().to_string_lossy().replace('\\', "/");
            Project { name, root, files, capabilities }
        })
        .collect()
}

/// The listeners of the shown screen and its sheets: `(node, event, handler)`.
fn listeners(tree: &Tree) -> Vec<(NodeId, u16, u32)> {
    let mut out = Vec::new();
    let mut stack: Vec<NodeId> = tree.current_root().map(|n| vec![n.id]).unwrap_or_default();
    // Other screens too: a nav click shows them.
    stack.extend(tree.screens().map(|(_, id)| id));
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(n) = tree.get(id) else { continue };
        for &(ev, h) in &n.listeners {
            out.push((id, ev, h));
        }
        stack.extend(n.children.iter().copied());
    }
    out
}

const WORDS: &[&str] = &["", "a", "Hello", "42", "-3.5", "1e3", "=A1+B2", "=SUM(A0:B3)", "=A0", "x y z", "2026-10-07", "31.12.2027", "\u{1F600}", "  ", "NaN"];
const KEYS: &[&str] = &["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", " ", "Enter", "Escape", "w", "s", "a"];

/// A plausible value for `ev` on node `n`, like a host would send.
fn value_for(rng: &mut Rng, tree: &Tree, id: NodeId, ev: u16) -> Value {
    let n = tree.get(id).unwrap();
    match ev {
        event::CHANGE | event::SUBMIT => match n.kind {
            Some(ControlKind::Toggle | ControlKind::Checkbox | ControlKind::Sheet | ControlKind::Dialog | ControlKind::Menu) => Value::Bool(rng.below(2) == 1),
            Some(ControlKind::Slider | ControlKind::NumberField) => Value::Number(rng.pick(&[0.0, 1.0, 5.5, 50.0, 100.0, -1.0, 1e6])),
            Some(ControlKind::Picker) | Some(ControlKind::Tabs) => {
                let list = n.str_prop(prop::OPTIONS).or(n.str_prop(prop::ITEMS)).unwrap_or("");
                let opts: Vec<&str> = list.split('\u{1F}').collect();
                Value::Str(rng.pick(&opts).to_owned())
            }
            _ => Value::Str(rng.pick(WORDS).to_owned()),
        },
        event::KEY_DOWN | event::KEY_UP => Value::Str(rng.pick(KEYS).to_owned()),
        event::POINTER_DOWN | event::POINTER_MOVE | event::POINTER_UP | event::CONTEXT_MENU => {
            Value::List(vec![Value::Number(rng.below(400) as f64), Value::Number(rng.below(300) as f64)])
        }
        _ => Value::Null,
    }
}

struct Run<'a> {
    name: &'a str,
    guest: Guest,
    tree: Tree,
    clock: Instant,
    last: Vec<String>,
    errors: usize,
}

impl Run<'_> {
    fn apply(&mut self, commits: anyhow::Result<Vec<Vec<u8>>>, step: usize) {
        let commits = commits.unwrap_or_else(|e| {
            panic!("{}: the app stopped at step {step}: {e:#}\nlast events:\n  {}\nlogs: {:?}", self.name, self.last.join("\n  "), self.guest.take_logs())
        });
        for c in commits {
            let errors = self.tree.apply(&c).unwrap_or_else(|e| panic!("{}: a malformed commit at step {step}: {e}", self.name));
            assert!(errors.is_empty(), "{}: op errors at step {step}: {errors:?}\nlast events:\n  {}", self.name, self.last.join("\n  "));
        }
        self.errors += self.guest.take_errors().len();
        let _ = self.guest.take_logs();
    }

    fn note(&mut self, what: String) {
        self.last.push(what);
        if self.last.len() > 8 {
            self.last.remove(0);
        }
    }

    /// One random action; most are UI events.
    fn step(&mut self, rng: &mut Rng, step: usize) {
        match rng.below(20) {
            0 | 1 => {
                // Time passes: due timers fire.
                self.clock += Duration::from_millis(rng.pick(&[16, 100, 1000, 5000]));
                let r = self.guest.fire_due_timers(self.clock);
                self.note("timers".into());
                self.apply(r, step);
            }
            2 if self.guest.wants_frames() => {
                self.clock += Duration::from_millis(16);
                let r = self.guest.fire_frame(self.clock);
                self.note("frame".into());
                self.apply(r, step);
            }
            3 => {
                // Answer a dialog, and deliver finished requests.
                if let Some(d) = self.guest.pending_dialogs().first().map(|d| d.id) {
                    let v = rng.pick(&[Value::Null, Value::Bool(true), Value::Bool(false), Value::Str("ok".into())]);
                    self.note(format!("answer dialog {d} {v:?}"));
                    let r = self.guest.answer_dialog(d, v);
                    self.apply(r, step);
                }
                for (id, v) in self.guest.poll_net_results() {
                    let r = self.guest.answer_dialog(id, v);
                    self.apply(r, step);
                }
            }
            4 => {
                // Another screen, as a nav click does (no app event).
                let screens: Vec<u32> = self.tree.primary_screens().map(|(s, _)| s).collect();
                if !screens.is_empty() {
                    self.tree.current_screen = rng.pick(&screens);
                }
            }
            _ => {
                let ls = listeners(&self.tree);
                if ls.is_empty() {
                    return;
                }
                let (id, ev, handler) = rng.pick(&ls);
                let value = value_for(rng, &self.tree, id, ev);
                let kind = self.tree.get(id).and_then(|n| n.kind);
                self.note(format!("{kind:?} #{id} event {ev} {value:?}"));
                let mut w = Writer::new();
                w.event(&Event::Ui { handler, event: ev, value });
                let r = self.guest.on_event(w.as_bytes());
                self.apply(r, step);
            }
        }
    }
}

fn monkey(project: &Project, runner: &Runner, stress: bool, steps: usize) {
    let fs = MemFs(project.files.clone());
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &project.capabilities).unwrap();
    let artifact = artifact.unwrap_or_else(|| panic!("{} does not build: {:?}", project.name, front.diags.iter().map(|d| front.sources.render(d)).collect::<Vec<_>>()));
    // Every declared capability except the network: no test reaches it.
    let declared: Vec<String> = project.capabilities.iter().filter(|c| !c.starts_with("net")).cloned().collect();
    let mut guest = runner
        .load_with_policy(&artifact.component, Limits::default(), Policy::new(declared), Kv::in_memory(), Box::new(MemoryClipboard::default()))
        .unwrap();
    let space = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/monkey-files").join(project.name.replace('/', "-"));
    let _ = std::fs::remove_dir_all(&space);
    guest.set_files(plinth_runner_wasmtime::files::Files::open(space, 1 << 20));
    let args = if stress { plinth_protocol::init_arg::one(plinth_protocol::init_arg::GC_STRESS, &[]) } else { Vec::new() };
    let mut seed_rec = plinth_protocol::init_arg::one(plinth_protocol::init_arg::RANDOM_SEED, &7u64.to_le_bytes());
    seed_rec.extend(args);
    let commits = guest.init(&seed_rec);
    let mut run = Run { name: &project.name, guest, tree: Tree::new(), clock: Instant::now(), last: Vec::new(), errors: 0 };
    run.apply(commits, 0);
    let mut rng = Rng(0x40_4E_4B ^ project.name.len() as u64 ^ u64::from(stress));
    let mut half_memory = 0;
    for step in 1..=steps {
        run.step(&mut rng, step);
        if step == steps / 2 {
            half_memory = run.guest.memory_bytes();
        }
        if std::env::var("PLINTH_MONKEY_TRACE").is_ok() && step % (steps / 10).max(1) == 0 {
            eprintln!("  step {step}: memory {} KiB", run.guest.memory_bytes() >> 10);
        }
    }
    let end_memory = run.guest.memory_bytes();
    eprintln!(
        "{}{}: {steps} steps, {} uncaught app error(s), memory {} KiB at half, {} KiB at end, {} nodes",
        project.name,
        if stress { " (GC stress)" } else { "" },
        run.errors,
        half_memory >> 10,
        end_memory >> 10,
        run.tree.len()
    );
    // The memory only grows. The second half may grow it a little (a list
    // that the monkey made longer), but not without a limit: at most a
    // quarter more, and at least 256 KiB of room.
    let allowed = half_memory + (half_memory / 4).max(256 << 10);
    assert!(end_memory <= allowed, "{}: memory grew from {} KiB to {} KiB in the second half", project.name, half_memory >> 10, end_memory >> 10);
}

#[test]
fn every_example_survives_random_use() {
    let steps: usize = std::env::var("PLINTH_MONKEY_STEPS").ok().and_then(|v| v.parse().ok()).unwrap_or(400);
    let only = std::env::var("PLINTH_MONKEY_APP").ok();
    let runner = Runner::new().unwrap();
    let projects = projects();
    let mut ran = 0;
    for p in &projects {
        if only.as_deref().is_some_and(|o| !p.name.contains(o)) {
            continue;
        }
        let _ = &p.root;
        monkey(p, &runner, false, steps);
        monkey(p, &runner, true, steps / 4);
        ran += 1;
    }
    assert!(ran > 0, "no app matched");
}
