//! e2e: `plinth build --release` optimizes the app module with `wasm-opt`
//! (SPEC.md §5.1 step 10, `docs/PRODUCTION.md` T7). The release module is
//! smaller, links into the built-in core, behaves the same as the debug
//! module (the same op stream for the same events), and the build is
//! deterministic.

use plinth_protocol::{Event, NodeId, Value, Writer, event};
use plinth_runner_wasmtime::{Guest, Limits, Runner};
use plinth_ui::tree::Tree;
use std::path::{Path, PathBuf};
use std::process::Command;

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(name)
}

fn scratch(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("plinth-release-{test}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs `plinth build <example> --out <out> [--release]` and returns the
/// package bytes.
fn build(name: &str, out: &Path, release: bool) -> Vec<u8> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_plinth"));
    cmd.arg("build").arg(example(name)).arg("--out").arg(out);
    if release {
        cmd.arg("--release");
    }
    let o = cmd.output().expect("run plinth build");
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(o.status.success(), "plinth build {name} failed: {stderr}");
    assert!(!stderr.contains("not optimized"), "wasm-opt was not used for {name}: {stderr}");
    std::fs::read(out).unwrap()
}

/// The `app.wasm` of a package.
fn app_module(plnt: &[u8]) -> Vec<u8> {
    plinth_package::Package::read(plnt).unwrap().component
}

struct Run {
    guest: Guest,
    tree: Tree,
    _runner: Runner,
}

impl Run {
    /// Links `app` into the built-in core the same way a host does and
    /// starts it. Returns the run and the op stream of `init`.
    fn start(app: &[u8]) -> (Self, Vec<Vec<u8>>) {
        let component = plinth_link::split::link_app(plinth_compiler::link::runtime(), app).expect("link the app module");
        let runner = Runner::new().unwrap();
        let mut guest = runner.load(&component, Limits::default()).unwrap();
        let seed = plinth_protocol::init_arg::one(plinth_protocol::init_arg::RANDOM_SEED, &1u64.to_le_bytes());
        let commits = guest.init(&seed).unwrap();
        let mut run = Self { guest, tree: Tree::new(), _runner: runner };
        run.apply(&commits);
        (run, commits)
    }

    fn apply(&mut self, commits: &[Vec<u8>]) {
        for commit in commits {
            let errors = self.tree.apply(commit).unwrap();
            assert!(errors.is_empty(), "op errors: {errors:?}");
        }
    }

    /// The nodes of the current screens that listen for a press, in tree
    /// order.
    fn pressable(&self) -> Vec<NodeId> {
        let mut found = Vec::new();
        let mut stack: Vec<NodeId> = self.tree.screens().map(|(_, id)| id).collect();
        stack.reverse();
        while let Some(id) = stack.pop() {
            let Some(node) = self.tree.get(id) else { continue };
            if node.handler(event::PRESS).is_some() {
                found.push(id);
            }
            stack.extend(node.children.iter().rev());
        }
        found
    }

    fn press(&mut self, node: NodeId) -> Vec<Vec<u8>> {
        let handler = self.tree.get(node).and_then(|n| n.handler(event::PRESS)).unwrap();
        let mut w = Writer::new();
        w.event(&Event::Ui { handler, event: event::PRESS, value: Value::Null });
        let commits = self.guest.on_event(w.as_bytes()).unwrap();
        self.apply(&commits);
        commits
    }
}

/// The debug and the release module give the same ops at the start and
/// after each of `steps` presses (each press takes the next pressable node
/// of the debug run's tree).
fn same_behavior(name: &str, debug: &[u8], release: &[u8], steps: usize) {
    let (mut d, d_init) = Run::start(debug);
    let (mut r, r_init) = Run::start(release);
    assert_eq!(d_init, r_init, "{name}: init gives other ops");
    let mut ops = 0;
    for step in 0..steps {
        let nodes = d.pressable();
        assert_eq!(nodes, r.pressable(), "{name}: other pressable nodes at step {step}");
        let Some(&node) = nodes.get(step % nodes.len().max(1)) else { break };
        let commits = d.press(node);
        assert_eq!(commits, r.press(node), "{name}: press {step} gives other ops");
        ops += commits.iter().map(Vec::len).sum::<usize>();
    }
    eprintln!("{name}: {steps} presses, {ops} op bytes, the same on both");
    assert!(ops > 0, "{name}: the presses changed nothing; the test checks nothing");
}

#[test]
fn release_build_is_smaller_and_behaves_the_same() {
    let dir = scratch("same");
    for name in ["counter", "todo", "calculator", "budget", "7guis/crud", "pong"] {
        let file = name.replace('/', "-");
        let debug = app_module(&build(name, &dir.join(format!("{file}.plnt")), false));
        let release = app_module(&build(name, &dir.join(format!("{file}.release.plnt")), true));
        eprintln!("{name}: app {} B debug, {} B release", debug.len(), release.len());
        assert!(release.len() < debug.len(), "{name}: the release module is not smaller");
        // The core version that the app needs is the same.
        assert_eq!(plinth_link::split::runtime_field(&debug), plinth_link::split::runtime_field(&release));
        same_behavior(name, &debug, &release, 24);
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn release_build_is_deterministic() {
    let dir = scratch("same-bytes");
    // The largest example app.
    let first = build("hub", &dir.join("a.plnt"), true);
    let second = build("hub", &dir.join("b.plnt"), true);
    assert!(first == second, "two release builds of the same source give other bytes");
    std::fs::remove_dir_all(&dir).ok();
}
