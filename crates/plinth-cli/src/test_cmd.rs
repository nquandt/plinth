//! `plinth test` (docs/TESTING.md): builds the test build of an app and runs
//! each test in a new app instance on the wasmtime runner, with no window.
//!
//! The test code runs in the guest (`plinth:test`, compiled into the test
//! build). It sends its steps to this runner as `dev.log` lines that start
//! with `MARK`; the runner keeps the UI tree, does the step through the
//! normal event path, and answers with a `change` event on the text field of
//! the harness screen. Nothing in the core or the WIT is special to tests.

use anyhow::{Context as _, Result};
use plinth_package::ProjectConfig;
use plinth_protocol::{ControlKind, Event, NodeId, Value, Writer, event, prop};
use plinth_runner_wasmtime::kv::Kv;
use plinth_runner_wasmtime::policy::Policy;
use plinth_runner_wasmtime::{Guest, Limits, MemoryClipboard, Runner};
use plinth_ui::tree::{Node, Tree};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const MARK: &str = "\u{1}plinth:test\u{1}";
const SEP: char = '\u{1f}';
const STEP_SEP: char = '\u{1e}';
const PART_SEP: char = '\u{1d}';
const VIEW_TITLE: &str = "plinth:test view";
const HARNESS_TITLE: &str = "plinth:test harness";

/// The epoch time of the test clock: 2026-01-01T00:00:00Z.
const TEST_EPOCH_MS: u64 = 1_767_225_600_000;
/// The most time that one test can take (wall clock).
const TEST_TIME_LIMIT: Duration = Duration::from_secs(30);
/// `settle` runs the timers that are due in this much test time.
const SETTLE_SPAN: Duration = Duration::from_secs(60);
/// A frame of a 60 Hz display, for `onFrame` while the test clock moves.
const FRAME: Duration = Duration::from_nanos(16_666_667);

/// Finds the test files of the project: `app/**/*.test.ts(x)`, sorted.
pub fn find_test_files(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.join("app")];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(rel) = p.strip_prefix(dir) {
                let rel = rel.to_string_lossy().replace('\\', "/");
                if plinth_compiler::driver::is_test_file(&rel) {
                    out.push(rel);
                }
            }
        }
    }
    out.sort();
    out
}

/// One test that the build registered.
struct TestCase {
    name: String,
    at: String,
}

/// Why a test failed.
struct Failure {
    message: String,
    at: String,
    /// The screen as text, or other lines that help.
    detail: String,
}

impl Failure {
    fn new(message: impl Into<String>, at: &str) -> Self {
        Self {
            message: message.into(),
            at: at.to_owned(),
            detail: String::new(),
        }
    }
}

/// `plinth test [dir] [--filter text]`. Returns `true` when all tests pass.
pub fn run(dir: &Path, filter: Option<&str>) -> Result<bool> {
    let config = crate::ensure_project(dir)?;
    let tests = find_test_files(dir);
    if tests.is_empty() {
        println!("no test files (app/**/*.test.ts or *.test.tsx)");
        return Ok(true);
    }
    let caps: Vec<String> = config.capabilities.iter().map(|c| c.name.clone()).collect();
    let fs = plinth_compiler::driver::DiskFs {
        root: dir.to_path_buf(),
    };
    let (front, artifact) = plinth_compiler::compile_tests(&fs, &caps, &tests)?;
    let (e, w) = crate::print_diags(&front);
    let Some(artifact) = artifact else {
        eprintln!("{}", crate::summary(e, w));
        return Ok(false);
    };
    let runner = Runner::new()?;
    let kit = Kit {
        component: artifact.component,
        config,
        caps,
        runner,
    };

    // A first instance lists the tests (each `test(...)` sends a record).
    let mut first = kit.session(0)?;
    let cases: Vec<TestCase> = first
        .records
        .drain(..)
        .filter_map(|r| {
            let f: Vec<&str> = r.split(SEP).collect();
            (f.first() == Some(&"test")).then(|| TestCase {
                name: f.get(1).unwrap_or(&"").to_string(),
                at: f.get(2).unwrap_or(&"").to_string(),
            })
        })
        .collect();
    drop(first);
    if cases.is_empty() {
        println!("no tests: the test files call `test(...)` nowhere");
        return Ok(true);
    }

    let (mut passed, mut failed, mut skipped) = (0, 0, 0);
    for (i, case) in cases.iter().enumerate() {
        if filter.is_some_and(|f| !case.name.contains(f)) {
            skipped += 1;
            continue;
        }
        let started = Instant::now();
        let outcome = kit.session(i).map(|mut s| s.run(i));
        let ms = started.elapsed().as_millis();
        match outcome {
            Ok(Ok(())) => {
                passed += 1;
                println!("  ok  {} ({ms} ms)", case.name);
            }
            Ok(Err(f)) => {
                failed += 1;
                print_failure(&case.name, ms, &f, &case.at);
            }
            Err(e) => {
                failed += 1;
                print_failure(
                    &case.name,
                    ms,
                    &Failure::new(format!("{e:#}"), ""),
                    &case.at,
                );
            }
        }
    }
    let mut summary = format!("{passed} passed, {failed} failed");
    if skipped > 0 {
        let _ = write!(summary, ", {skipped} not run (--filter)");
    }
    println!("{summary}");
    Ok(failed == 0)
}

fn print_failure(name: &str, ms: u128, f: &Failure, test_at: &str) {
    println!("FAIL  {name} ({ms} ms)");
    for line in f.message.lines() {
        println!("      {line}");
    }
    if !f.at.is_empty() {
        println!("      at {}", f.at);
    } else if !test_at.is_empty() {
        println!("      in the test at {test_at}");
    }
    if !f.detail.is_empty() {
        for line in f.detail.lines() {
            println!("      {line}");
        }
    }
}

struct Kit {
    component: Vec<u8>,
    config: ProjectConfig,
    caps: Vec<String>,
    runner: Runner,
}

impl Kit {
    /// A new app instance with a test clock, an in-memory store and a new
    /// private space: no state goes from one test to the next.
    fn session(&self, index: usize) -> Result<Session> {
        let mut policy = Policy::new(self.caps.clone());
        // No network in tests: a `fetch` gets `denied:unsupported`.
        policy.mark_unsupported("net");
        let mut guest = self.runner.load_with_policy(
            &self.component,
            Limits::default(),
            policy,
            Kv::in_memory(),
            Box::new(MemoryClipboard::default()),
        )?;
        let space = test_space(&self.config.id, index)?;
        guest.set_files(plinth_runner_wasmtime::files::Files::open(
            space.clone(),
            64 << 20,
        ));
        guest.set_locale("en-US");
        guest.set_fake_timezone_offset(0);
        let clock = guest.use_test_clock(TEST_EPOCH_MS);
        let seed = plinth_protocol::init_arg::one(
            plinth_protocol::init_arg::RANDOM_SEED,
            &1u64.to_le_bytes(),
        );
        let mut s = Session {
            guest,
            tree: Tree::new(),
            clock,
            scope: Scope::None,
            records: Vec::new(),
            app_logs: Vec::new(),
            space,
            harness: 0,
            view: None,
            app_screen: 0,
        };
        let commits = s.guest.init(&seed).context("start the test build")?;
        s.apply(commits);
        s.collect_logs();
        s.find_harness()?;
        Ok(s)
    }
}

/// A new, empty folder for the private space of one test.
fn test_space(app_id: &str, index: usize) -> Result<PathBuf> {
    static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "plinth-test-{}-{}-{index}-{n}",
        std::process::id(),
        app_id.replace(['/', '\\'], "_")
    ));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).ok();
    }
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[derive(Clone, Copy, PartialEq)]
enum Scope {
    /// Before `render` or `renderApp`.
    None,
    /// The view screen of `render`.
    View,
    /// The screen of the app that is shown.
    App,
}

struct Session {
    guest: Guest,
    tree: Tree,
    clock: Instant,
    scope: Scope,
    /// The records of `plinth:test` (without `MARK`), in order.
    records: Vec<String>,
    /// The other `console.log` lines of the app.
    app_logs: Vec<String>,
    space: PathBuf,
    /// The text field of the harness screen.
    harness: NodeId,
    /// The screen index of the view screen.
    view: Option<u32>,
    /// The first primary screen of the app.
    app_screen: u32,
}

impl Drop for Session {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.space).ok();
    }
}

impl Session {
    fn apply(&mut self, commits: Vec<Vec<u8>>) {
        for c in commits {
            // A bad op is skipped, as a host does.
            let _ = self.tree.apply(&c);
        }
    }

    fn collect_logs(&mut self) {
        for line in self.guest.take_logs() {
            match line.strip_prefix(MARK) {
                Some(r) => self.records.push(r.to_owned()),
                None => self.app_logs.push(line),
            }
        }
    }

    fn find_harness(&mut self) -> Result<()> {
        let mut harness = None;
        for (index, root) in self.tree.screens() {
            let title = self
                .tree
                .get(root)
                .and_then(|n| n.str_prop(prop::TITLE))
                .unwrap_or_default();
            if title == VIEW_TITLE {
                self.view = Some(index);
            } else if title == HARNESS_TITLE {
                harness = self
                    .tree
                    .get(root)
                    .and_then(|n| n.children.first().copied());
            }
        }
        self.app_screen = self
            .tree
            .primary_screens()
            .map(|(i, _)| i)
            .find(|i| Some(*i) != self.view)
            .unwrap_or(0);
        self.harness = harness.context("the test build has no harness screen")?;
        Ok(())
    }

    /// Sends one `ui` event through the normal event path and applies what
    /// the guest commits.
    fn fire(&mut self, node: NodeId, ev: u16, value: Value) -> Result<(), Failure> {
        let Some(handler) = self.tree.get(node).and_then(|n| n.handler(ev)) else {
            return Err(Failure::new("the node has no handler for this event", ""));
        };
        let mut w = Writer::new();
        w.event(&Event::Ui {
            handler,
            event: ev,
            value,
        });
        let result = self.guest.on_event(w.as_bytes());
        self.after_call(result)
    }

    fn after_call(&mut self, result: Result<Vec<Vec<u8>>>) -> Result<(), Failure> {
        let commits = result.map_err(|e| Failure::new(format!("the app stopped: {e:#}"), ""))?;
        self.apply(commits);
        self.collect_logs();
        let errors = self.guest.take_errors();
        if let Some(e) = errors.first() {
            return Err(Failure::new(format!("the app reported an error: {e}"), ""));
        }
        Ok(())
    }

    fn send(&mut self, command: &str) -> Result<(), Failure> {
        self.fire(self.harness, event::CHANGE, Value::Str(command.to_owned()))
    }

    /// Runs the test at `index` to its end.
    fn run(&mut self, index: usize) -> Result<(), Failure> {
        self.records.clear();
        let started = Instant::now();
        self.send(&format!("run{SEP}{index}"))?;
        loop {
            if started.elapsed() > TEST_TIME_LIMIT {
                return Err(Failure::new(
                    format!("the test took more than {} s", TEST_TIME_LIMIT.as_secs()),
                    "",
                ));
            }
            let records = std::mem::take(&mut self.records);
            let mut reply = None;
            for r in records {
                let f: Vec<&str> = r.split(SEP).collect();
                let field = |i: usize| f.get(i).copied().unwrap_or("");
                match field(0) {
                    "test" => {}
                    "render" => {
                        self.scope = Scope::View;
                        if let Some(v) = self.view {
                            self.tree.current_screen = v;
                        }
                        reply = Some(String::new());
                    }
                    "renderApp" => {
                        self.scope = Scope::App;
                        self.tree.current_screen = self.app_screen;
                        reply = Some(String::new());
                    }
                    "screen" => {
                        let (title, at) = (field(1), field(2));
                        let primary: Vec<(u32, String)> = self
                            .tree
                            .primary_screens()
                            .filter(|(i, _)| Some(*i) != self.view)
                            .map(|(i, id)| {
                                (
                                    i,
                                    self.tree
                                        .get(id)
                                        .and_then(|n| n.str_prop(prop::TITLE))
                                        .unwrap_or_default()
                                        .to_owned(),
                                )
                            })
                            .collect();
                        let Some((index, _)) = primary.iter().find(|(_, t)| t == title) else {
                            let titles: Vec<String> =
                                primary.iter().map(|(_, t)| format!("{t:?}")).collect();
                            return Err(Failure::new(
                                format!(
                                    "the app has no primary screen {title:?}; it has {}",
                                    titles.join(", ")
                                ),
                                at,
                            ));
                        };
                        self.tree.current_screen = *index;
                        self.scope = Scope::App;
                        reply = Some(String::new());
                    }
                    "act" => {
                        let (action, path, value, at, desc) =
                            (field(1), field(2), field(3), field(4), field(5));
                        self.act(action, path, value, desc).map_err(|mut e| {
                            if e.at.is_empty() {
                                e.at = at.to_owned();
                            }
                            e
                        })?;
                        reply = Some(String::new());
                    }
                    "read" => {
                        let (path, at, desc) = (field(1), field(2), field(3));
                        let text = if path.is_empty() {
                            if self.scope_root().is_none() {
                                return Err(Failure::new(
                                    "call `render(...)` or `renderApp()` before this step",
                                    at,
                                ));
                            }
                            self.screen_text(usize::MAX)
                        } else {
                            let node = self.one(path, desc).map_err(|e| self.with_screen(e, at))?;
                            self.text_of(node)
                        };
                        reply = Some(text);
                    }
                    "expect" => {
                        let (path, matcher, arg, negate, at, desc) = (
                            field(1),
                            field(2),
                            field(3),
                            field(4) == "not",
                            field(5),
                            field(6),
                        );
                        self.expect(path, matcher, arg, negate, desc)
                            .map_err(|e| self.with_screen(Failure::new(e.message, at), at))?;
                    }
                    "fail" => return Err(Failure::new(field(1), field(2))),
                    "tick" => {
                        let ms: f64 = field(1).parse().unwrap_or(0.0);
                        self.advance(Duration::from_secs_f64(ms.max(0.0) / 1000.0))?;
                        reply = Some(String::new());
                    }
                    "settle" => {
                        self.settle()?;
                        reply = Some(String::new());
                    }
                    "done" => return Ok(()),
                    "error" => {
                        return Err(Failure::new(format!("the test threw {}", field(1)), ""));
                    }
                    other => {
                        return Err(Failure::new(format!("unknown test record `{other}`"), ""));
                    }
                }
            }
            match reply {
                Some(r) => self.send(&format!("ok{SEP}{r}"))?,
                None => {
                    return Err(Failure::new(
                        "the test did not finish: it waits for something that does not come\n(a timer or a host answer: use `await tick(ms)` or `await s.settle()`)",
                        "",
                    ));
                }
            }
        }
    }

    /// The root of the screen that the queries search.
    fn scope_root(&self) -> Option<NodeId> {
        match self.scope {
            Scope::None => None,
            Scope::View => self
                .view
                .and_then(|v| self.tree.screens().find(|(i, _)| *i == v).map(|(_, id)| id)),
            Scope::App => self.tree.current_root().map(|n| n.id),
        }
    }

    fn with_screen(&self, mut f: Failure, at: &str) -> Failure {
        if f.at.is_empty() {
            f.at = at.to_owned();
        }
        if f.detail.is_empty() && self.scope_root().is_some() {
            f.detail = format!("the screen:\n{}", indent(&self.screen_text(60)));
        }
        f
    }

    /// The screen of the queries as indented text, at most `max` lines. On
    /// the view screen, only what `render` shows.
    fn screen_text(&self, max: usize) -> String {
        let Some(root) = self.scope_root() else {
            return String::new();
        };
        match self.scope {
            Scope::View => {
                let kids = self
                    .tree
                    .get(root)
                    .map(|n| n.children.clone())
                    .unwrap_or_default();
                kids.iter().map(|k| self.outline(*k, 0, max)).collect()
            }
            _ => self.outline(root, 0, max),
        }
    }

    /// The nodes that `path` finds. Each step but the last must find one
    /// node.
    fn find(&self, path: &str, desc: &str) -> Result<Vec<NodeId>, Failure> {
        let root = self.scope_root().ok_or_else(|| {
            Failure::new("call `render(...)` or `renderApp()` before this step", "")
        })?;
        let mut parents = vec![root];
        let steps: Vec<&str> = if path.is_empty() {
            Vec::new()
        } else {
            path.split(STEP_SEP).collect()
        };
        for (i, step) in steps.iter().enumerate() {
            let parts: Vec<&str> = step.split(PART_SEP).collect();
            let (kind, role, name) = (
                parts.first().copied().unwrap_or(""),
                parts.get(1).copied().unwrap_or(""),
                parts.get(2).copied().unwrap_or(""),
            );
            let parent = match parents.as_slice() {
                [one] => *one,
                [] => {
                    return Err(Failure::new(
                        format!("nothing matches {}", step_desc(desc, i)),
                        "",
                    ));
                }
                many => return Err(self.ambiguous(&step_desc(desc, i), many)),
            };
            let mut found = Vec::new();
            self.walk(parent, &mut |n| {
                if n.id != parent && matches(n, kind, role, name) {
                    found.push(n.id);
                }
            });
            parents = found;
        }
        Ok(parents)
    }

    /// The one node that `path` finds.
    fn one(&self, path: &str, desc: &str) -> Result<NodeId, Failure> {
        match self.find(path, desc)?.as_slice() {
            [one] => Ok(*one),
            [] => Err(Failure::new(format!("nothing matches {desc}"), "")),
            many => Err(self.ambiguous(desc, many)),
        }
    }

    fn ambiguous(&self, desc: &str, nodes: &[NodeId]) -> Failure {
        let mut m = format!(
            "{desc} matches {} nodes; make the query more exact (for example `s.row(\"...\").get(\"...\")`):",
            nodes.len()
        );
        for id in nodes {
            let _ = write!(m, "\n  {}", self.describe_in_context(*id));
        }
        Failure::new(m, "")
    }

    /// Calls `f` on `root` and each node under it that the user can see (a
    /// closed sheet, dialog or menu hides its nodes).
    fn walk(&self, root: NodeId, f: &mut dyn FnMut(&Node)) {
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let Some(n) = self.tree.get(id) else { continue };
            if id != root && is_closed(n) {
                continue;
            }
            f(n);
            stack.extend(n.children.iter().rev());
        }
    }

    fn act(&mut self, action: &str, path: &str, value: &str, desc: &str) -> Result<(), Failure> {
        let node = self.one(path, desc).map_err(|e| self.with_screen(e, ""))?;
        let n = self.tree.get(node).cloned().expect("found above");
        let what = self.describe(&n);
        if n.bool_prop(prop::DISABLED) {
            return Err(self.with_screen(
                Failure::new(format!("cannot {action} {what}: it is disabled"), ""),
                "",
            ));
        }
        let kind = n.kind;
        let no = |s: &Self, why: &str| {
            Err(s.with_screen(
                Failure::new(format!("cannot {action} {what}: {why}"), ""),
                "",
            ))
        };
        match action {
            "press" => {
                if n.handler(event::PRESS).is_none() {
                    return no(self, "it has no `onPress`");
                }
                self.fire(node, event::PRESS, Value::Null)
            }
            "type" => {
                let v = match kind {
                    Some(ControlKind::TextField | ControlKind::TextArea) => {
                        Value::Str(value.to_owned())
                    }
                    Some(ControlKind::NumberField) => match value.trim().parse::<f64>() {
                        Ok(x) => Value::Number(x),
                        Err(_) => return no(self, "a number field takes a number"),
                    },
                    _ => return no(self, "it is not a text field"),
                };
                if n.handler(event::CHANGE).is_none() {
                    return no(self, "it has no `onChange` or bound `value`");
                }
                self.tree.set_local_prop(node, prop::VALUE, v.clone());
                self.fire(node, event::CHANGE, v)
            }
            "submit" => {
                if n.handler(event::SUBMIT).is_none() {
                    return no(self, "it has no `onSubmit`");
                }
                let v = n
                    .prop(prop::VALUE)
                    .cloned()
                    .unwrap_or(Value::Str(String::new()));
                self.fire(node, event::SUBMIT, v)
            }
            "toggle" => {
                if !matches!(kind, Some(ControlKind::Toggle | ControlKind::Checkbox)) {
                    return no(self, "it is not a toggle or a checkbox");
                }
                if n.handler(event::CHANGE).is_none() {
                    return no(self, "it has no `onChange` or bound `value`");
                }
                let v = Value::Bool(!n.bool_prop(prop::VALUE));
                self.tree.set_local_prop(node, prop::VALUE, v.clone());
                self.fire(node, event::CHANGE, v)
            }
            "select" => {
                let options = match kind {
                    Some(ControlKind::Picker) => n.str_prop(prop::OPTIONS),
                    Some(ControlKind::Tabs) => n.str_prop(prop::ITEMS),
                    _ => return no(self, "it is not a picker or tabs"),
                };
                let options: Vec<&str> = options.unwrap_or_default().split('\u{1f}').collect();
                if !options.contains(&value) {
                    return no(
                        self,
                        &format!(
                            "{value:?} is not one of its options ({})",
                            options.join(", ")
                        ),
                    );
                }
                if n.handler(event::CHANGE).is_none() {
                    return no(self, "it has no `onChange` or bound `value`");
                }
                let v = Value::Str(value.to_owned());
                self.tree.set_local_prop(node, prop::VALUE, v.clone());
                self.fire(node, event::CHANGE, v)
            }
            "moveUp" | "moveDown" => {
                let Some(list) = self
                    .tree
                    .get(n.parent)
                    .filter(|p| p.kind == Some(ControlKind::List))
                else {
                    return no(self, "it is not a row of a list");
                };
                if list.handler(event::REORDER).is_none() {
                    return no(self, "its list has no `onReorder`");
                }
                let (list_id, count) = (list.id, list.children.len());
                let from = list.children.iter().position(|c| *c == node).unwrap_or(0);
                let to = if action == "moveUp" {
                    from.checked_sub(1)
                } else {
                    Some(from + 1).filter(|t| *t < count)
                };
                let Some(to) = to else {
                    return no(
                        self,
                        if action == "moveUp" {
                            "it is already first in its list"
                        } else {
                            "it is already last in its list"
                        },
                    );
                };
                let v = Value::List(vec![Value::Int(from as i32), Value::Int(to as i32)]);
                self.fire(list_id, event::REORDER, v)
            }
            other => Err(Failure::new(format!("unknown action `{other}`"), "")),
        }
    }

    fn expect(
        &self,
        path: &str,
        matcher: &str,
        arg: &str,
        negate: bool,
        desc: &str,
    ) -> Result<(), Failure> {
        let not = if negate { "not." } else { "" };
        let shown_arg = match matcher {
            "toExist" | "toBeChecked" | "toBeDisabled" => String::new(),
            "toHaveCount" => arg.to_owned(),
            _ => format!("{arg:?}"),
        };
        let head = format!("expect({desc}).{not}{matcher}({shown_arg})");
        let fail = |why: String| Err(Failure::new(format!("{head}\n{why}"), ""));
        let nodes = self
            .find(path, desc)
            .map_err(|e| Failure::new(format!("{head}\n{}", e.message), ""))?;
        let (ok, why) = match matcher {
            "toExist" => (
                !nodes.is_empty(),
                if nodes.is_empty() {
                    format!("nothing matches {desc}")
                } else {
                    format!("{desc} matches {} node(s)", nodes.len())
                },
            ),
            "toHaveCount" => {
                let n: usize = arg.parse().unwrap_or(0);
                (
                    nodes.len() == n,
                    format!("{desc} matches {} node(s)", nodes.len()),
                )
            }
            _ => {
                let node = match nodes.as_slice() {
                    [one] => *one,
                    [] => return fail(format!("nothing matches {desc}")),
                    many => return fail(self.ambiguous(desc, many).message),
                };
                let n = self.tree.get(node).expect("found");
                match matcher {
                    "toHaveText" => {
                        let t = self.text_of(node);
                        (t == arg, format!("the text is {t:?}"))
                    }
                    "toContainText" => {
                        let t = self.text_of(node);
                        (t.contains(arg), format!("the text is {t:?}"))
                    }
                    "toBeChecked" => (
                        n.bool_prop(prop::VALUE),
                        format!(
                            "{} is {}",
                            self.describe(n),
                            if n.bool_prop(prop::VALUE) {
                                "on"
                            } else {
                                "off"
                            }
                        ),
                    ),
                    "toBeDisabled" => {
                        let d = n.bool_prop(prop::DISABLED);
                        (
                            d,
                            format!(
                                "{} is {}",
                                self.describe(n),
                                if d { "disabled" } else { "enabled" }
                            ),
                        )
                    }
                    other => return fail(format!("unknown matcher `{other}`")),
                }
            }
        };
        if ok == negate { fail(why) } else { Ok(()) }
    }

    /// The text of a node: its text content, its value, or its title or
    /// label.
    fn text_of(&self, id: NodeId) -> String {
        let Some(n) = self.tree.get(id) else {
            return String::new();
        };
        if let Some(t) = &n.text {
            return t.clone();
        }
        if let Some(Value::Str(v)) = n.prop(prop::VALUE) {
            return v.clone();
        }
        n.str_prop(prop::TITLE)
            .or_else(|| n.str_prop(prop::LABEL))
            .unwrap_or_default()
            .to_owned()
    }

    fn describe(&self, n: &Node) -> String {
        let role = n.kind.map(|k| k.name()).unwrap_or("node");
        match node_name(n) {
            Some(name) => format!("{role} {name:?}"),
            None => role.to_owned(),
        }
    }

    /// `button "Delete" in row "Buy milk"`.
    fn describe_in_context(&self, id: NodeId) -> String {
        let Some(n) = self.tree.get(id) else {
            return String::new();
        };
        let mut s = self.describe(n);
        let mut p = n.parent;
        while let Some(parent) = self.tree.get(p) {
            if matches!(
                parent.kind,
                Some(
                    ControlKind::Row
                        | ControlKind::Section
                        | ControlKind::Sheet
                        | ControlKind::Dialog
                )
            ) && node_name(parent).is_some()
            {
                let _ = write!(s, " in {}", self.describe(parent));
                break;
            }
            p = parent.parent;
        }
        s
    }

    /// The screen as indented text, at most `max` lines.
    fn outline(&self, root: NodeId, depth: usize, max: usize) -> String {
        let mut out = String::new();
        let mut lines = 0;
        self.outline_into(root, depth, max, &mut lines, &mut out);
        if lines > max {
            let _ = writeln!(out, "... ({} more lines)", lines - max);
        }
        out
    }

    fn outline_into(
        &self,
        id: NodeId,
        depth: usize,
        max: usize,
        lines: &mut usize,
        out: &mut String,
    ) {
        let Some(n) = self.tree.get(id) else { return };
        if depth > 0 && is_closed(n) {
            return;
        }
        *lines += 1;
        if *lines <= max {
            let mut line = format!("{}{}", "  ".repeat(depth), self.describe(n));
            if let Some(t) = &n.text
                && node_name(n).as_deref() != Some(t.as_str())
            {
                let _ = write!(line, " {t:?}");
            }
            match n.prop(prop::VALUE) {
                Some(Value::Str(v)) => {
                    let _ = write!(line, " value={v:?}");
                }
                Some(Value::Bool(b))
                    if matches!(n.kind, Some(ControlKind::Toggle | ControlKind::Checkbox)) =>
                {
                    line.push_str(if *b { " on" } else { " off" });
                }
                Some(Value::Number(x)) => {
                    let _ = write!(line, " value={x}");
                }
                _ => {}
            }
            if let Some(sub) = n.str_prop(prop::SUBTITLE) {
                let _ = write!(line, " subtitle={sub:?}");
            }
            if n.bool_prop(prop::DISABLED) {
                line.push_str(" (disabled)");
            }
            out.push_str(&line);
            out.push('\n');
        }
        for c in &n.children {
            self.outline_into(*c, depth + 1, max, lines, out);
        }
    }

    // -- Time --------------------------------------------------------------

    /// Moves the test clock on by `by` and runs each timer (and each frame
    /// of `onFrame`) at its time.
    fn advance(&mut self, by: Duration) -> Result<(), Failure> {
        let target = self.clock + by;
        let mut runs = 0;
        loop {
            runs += 1;
            if runs > 100_000 {
                return Err(Failure::new(
                    "too many timers: more than 100000 in one step",
                    "",
                ));
            }
            let timer = self.guest.next_timer_deadline().filter(|d| *d <= target);
            let frame = self
                .guest
                .wants_frames()
                .then_some(self.clock + FRAME)
                .filter(|d| *d <= target);
            let next = match (timer, frame) {
                (Some(a), Some(b)) => a.min(b),
                (a, b) => match a.or(b) {
                    Some(t) => t,
                    None => break,
                },
            };
            self.clock = self
                .guest
                .advance_test_clock(next.max(self.clock))
                .unwrap_or(next);
            let result = self.guest.fire_due_timers(self.clock);
            self.after_call(result)?;
            if frame == Some(next) {
                let result = self.guest.fire_frame(self.clock);
                self.after_call(result)?;
            }
            self.deliver()?;
        }
        self.clock = self.guest.advance_test_clock(target).unwrap_or(target);
        self.deliver()
    }

    /// Gives the guest the host answers that are ready.
    fn deliver(&mut self) -> Result<(), Failure> {
        for (id, value) in self.guest.poll_net_results() {
            let result = self.guest.answer_dialog(id, value);
            self.after_call(result)?;
        }
        Ok(())
    }

    /// Waits for the host answers (real time, at most 10 s) and runs the
    /// timers due in the next `SETTLE_SPAN` of test time, until nothing is
    /// left.
    fn settle(&mut self) -> Result<(), Failure> {
        let limit = self.clock + SETTLE_SPAN;
        let started = Instant::now();
        loop {
            self.deliver()?;
            if self.guest.open_host_requests() > 0 {
                if started.elapsed() > Duration::from_secs(10) {
                    return Err(Failure::new(
                        "settle: a host request did not finish in 10 s",
                        "",
                    ));
                }
                std::thread::sleep(Duration::from_millis(1));
                continue;
            }
            match self.guest.next_timer_deadline().filter(|d| *d <= limit) {
                Some(d) => {
                    let by = d.saturating_duration_since(self.clock);
                    self.advance(by)?;
                }
                None => return Ok(()),
            }
        }
    }
}

fn step_desc(desc: &str, i: usize) -> String {
    desc.split(" > ").take(i).collect::<Vec<_>>().join(" > ")
}

fn indent(s: &str) -> String {
    s.lines().map(|l| format!("  {l}\n")).collect()
}

/// A sheet, a dialog or a menu that is closed: the user cannot see what is
/// in it.
fn is_closed(n: &Node) -> bool {
    matches!(
        n.kind,
        Some(ControlKind::Sheet | ControlKind::Dialog | ControlKind::Menu)
    ) && !n.bool_prop(prop::VALUE)
}

/// The name of a node for a query: its label, title or text.
fn node_name(n: &Node) -> Option<String> {
    n.str_prop(prop::LABEL)
        .or_else(|| n.str_prop(prop::TITLE))
        .map(str::to_owned)
        .or_else(|| n.text.clone())
}

fn matches(n: &Node, kind: &str, role: &str, name: &str) -> bool {
    if !role.is_empty() && n.kind.map(|k| k.name()) != Some(role) {
        return false;
    }
    match kind {
        "name" => {
            n.str_prop(prop::LABEL) == Some(name)
                || n.str_prop(prop::TITLE) == Some(name)
                || n.text.as_deref() == Some(name)
        }
        "text" => n.text.as_deref() == Some(name),
        "role" => true,
        _ => false,
    }
}
