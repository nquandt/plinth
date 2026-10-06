//! Dogfood apps: compiles `examples/budget` and `examples/utility` and
//! drives their main flows in wasmtime, the same way `tests/e2e.rs` drives
//! the other examples. See `docs/GAPS.md` for what the compiler and stdlib
//! could not express along the way.

use plinth_compiler::driver::DiskFs;
use plinth_protocol::{ControlKind, Event, NodeId, Value, Writer, event, prop};
use plinth_runner_wasmtime::{Guest, Limits, Runner};
use plinth_ui::tree::{Node, Tree};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// The capability names declared in `<example>/plinth.toml`'s
/// `[[capabilities]]` tables (SPEC.md §11), read with a minimal parse so
/// this test crate does not need a `plinth-package` dev-dependency.
fn declared_capabilities(root: &std::path::Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(root.join("plinth.toml")) else { return Vec::new() };
    let Ok(value) = text.parse::<toml::Value>() else { return Vec::new() };
    value
        .get("capabilities")
        .and_then(|c| c.as_array())
        .map(|caps| caps.iter().filter_map(|c| c.get("name")?.as_str().map(str::to_owned)).collect())
        .unwrap_or_default()
}

fn build(name: &str) -> plinth_compiler::Artifact {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(name);
    let caps = declared_capabilities(&root);
    let fs = DiskFs { root };
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &caps).expect("compile");
    let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
    artifact.unwrap_or_else(|| panic!("{name} has errors:\n{}", diags.join("\n")))
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
        Self { guest, tree, _runner: runner }
    }

    fn fire(&mut self, node: NodeId, ev: u16, value: Value) -> Duration {
        let handler = self.tree.get(node).and_then(|n| n.handler(ev)).expect("node has a handler");
        let mut w = Writer::new();
        w.event(&Event::Ui { handler, event: ev, value });
        let start = Instant::now();
        let commits = self.guest.on_event(w.as_bytes());
        for log in self.guest.take_logs() {
            eprintln!("guest: {log}");
        }
        for commit in commits.unwrap() {
            let errors = self.tree.apply(&commit).unwrap();
            assert!(errors.is_empty(), "op errors: {errors:?}");
        }
        start.elapsed()
    }

    /// Fires every timer due by `now` and applies what the guest commits.
    fn fire_timers(&mut self, now: Instant) {
        let commits = self.guest.fire_due_timers(now).unwrap();
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

    /// Like `find`, but scoped to the currently displayed screen's subtree.
    /// Other primary screens, and closed sheets, stay mounted in the tree
    /// (SPEC.md §6.3), so a plain `find`/`one` can see nodes the user is not
    /// looking at; use this when a label repeats across screens.
    fn find_in_current(&self, kind: ControlKind, pred: impl Fn(&Node) -> bool) -> Vec<NodeId> {
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

    fn one_in_current(&self, kind: ControlKind, pred: impl Fn(&Node) -> bool) -> NodeId {
        let f = self.find_in_current(kind, pred);
        assert_eq!(f.len(), 1, "expected one {kind:?} in the current screen, found {}", f.len());
        f[0]
    }

    fn label(&self, label: &str) -> NodeId {
        self.one(ControlKind::Button, |n| n.str_prop(prop::LABEL) == Some(label))
    }

    fn text_of(&self, id: NodeId) -> String {
        self.tree.get(id).unwrap().text.clone().unwrap_or_default()
    }

    fn row_titles(&self) -> Vec<String> {
        let list = self.one(ControlKind::List, |n| n.children.iter().any(|c| self.tree.get(*c).and_then(|c| c.kind) == Some(ControlKind::Row)));
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

    fn row_trailings(&self) -> Vec<String> {
        let list = self.one(ControlKind::List, |n| n.children.iter().any(|c| self.tree.get(*c).and_then(|c| c.kind) == Some(ControlKind::Row)));
        self.tree
            .get(list)
            .unwrap()
            .children
            .iter()
            .filter_map(|r| self.tree.get(*r))
            .filter(|n| n.kind == Some(ControlKind::Row))
            .map(|n| n.str_prop(prop::TRAILING).unwrap_or_default().to_owned())
            .collect()
    }
}

#[test]
fn budget_add_filter_edit_delete_and_stats() {
    let art = build("budget");
    eprintln!("budget: core {} KiB, component {} KiB", art.core_size / 1024, art.component.len() / 1024);
    let mut h = Harness::start(&art.component);

    assert_eq!(h.tree.current_root().unwrap().str_prop(prop::TITLE), Some("Budget"));
    // Five seed transactions (SPEC.md starting data in `model.ts`).
    assert_eq!(h.row_titles().len(), 5);

    // Add a transaction through the "Add" sheet. Both the add and edit
    // sheets are mounted at once (closed, not absent; SPEC.md §6.3), so the
    // test picks fields by their sheet's children, not by label (both
    // sheets share the same field labels).
    h.fire(h.one(ControlKind::Action, |n| n.str_prop(prop::LABEL) == Some("Add")), event::PRESS, Value::Null);
    let add_sheet = h.one(ControlKind::Sheet, |n| n.str_prop(prop::TITLE) == Some("Add transaction"));
    let add_children = h.tree.get(add_sheet).unwrap().children.clone();
    h.fire(add_children[0], event::CHANGE, "2026-10-06".into()); // Date
    h.fire(add_children[1], event::CHANGE, Value::Number(-15.0)); // Amount
    h.fire(add_children[4], event::PRESS, Value::Null); // "Add" button
    assert_eq!(h.row_titles().len(), 6);
    assert!(h.row_trailings().contains(&"-15.00".to_string()));

    // Filter by category: only "Salary" matches the one seed income row.
    let filter = h.one(ControlKind::Picker, |n| {
        n.str_prop(prop::LABEL) == Some("Category") && n.str_prop(prop::VALUE) == Some("All")
    });
    h.fire(filter, event::CHANGE, "Salary".into());
    assert_eq!(h.row_trailings(), ["+2500.00"]);
    h.fire(filter, event::CHANGE, "All".into());
    assert_eq!(h.row_titles().len(), 6);

    // Search by note text.
    let search = h.one(ControlKind::TextField, |n| n.str_prop(prop::LABEL) == Some("Search"));
    h.fire(search, event::CHANGE, "rent".into());
    assert_eq!(h.row_trailings(), ["-900.00"]);
    h.fire(search, event::CHANGE, "".into());
    assert_eq!(h.row_titles().len(), 6);

    // Edit the new row, then delete it with the destructive button in the
    // edit sheet.
    let row = h.find(ControlKind::Row, |n| n.str_prop(prop::TRAILING) == Some("-15.00"))[0];
    h.fire(row, event::PRESS, Value::Null);
    let edit_sheet = h.one(ControlKind::Sheet, |n| n.str_prop(prop::TITLE) == Some("Edit transaction"));
    let edit_children = h.tree.get(edit_sheet).unwrap().children.clone();
    h.fire(edit_children[3], event::CHANGE, "Coffee".into()); // Note
    h.fire(edit_children[4], event::PRESS, Value::Null); // "Save" button
    assert_eq!(h.row_titles().len(), 6);

    h.fire(row, event::PRESS, Value::Null);
    h.fire(edit_children[5], event::PRESS, Value::Null); // "Delete" button
    assert_eq!(h.row_titles().len(), 5);

    // Navigate to the stats screen: it has one progress bar per category.
    h.fire(h.label("View statistics"), event::PRESS, Value::Null);
    assert_eq!(h.tree.current_root().unwrap().str_prop(prop::TITLE), Some("Statistics"));
    assert_eq!(h.find(ControlKind::Progress, |_| true).len(), 6);
    // The charts' `data` comes from `categories.map(...)`: six points each.
    // The monthly totals come from a `Map` read with `for…of`.
    let charts = h.find(ControlKind::Chart, |_| true);
    assert_eq!(charts.len(), 2);
    for chart in charts {
        let data = h.tree.get(chart).unwrap().str_prop(prop::DATA).unwrap_or_default().to_owned();
        assert_eq!(data.split('\u{1f}').count(), 6, "{data:?}");
        assert!(data.starts_with("Groceries\u{1}"), "{data:?}");
        // Spending charts: income (the Salary row) counts as 0, not 2500.
        let salary = data.split('\u{1f}').find(|p| p.starts_with("Salary\u{1}")).expect("a Salary point");
        let value: f64 = salary.split('\u{1}').nth(1).unwrap().parse().unwrap();
        assert_eq!(value, 0.0, "{data:?}");
    }
    assert!(!h.find(ControlKind::Text, |n| n.text.as_deref().is_some_and(|t| t.starts_with("2026-10: "))).is_empty());
    h.fire(h.label("Back to transactions"), event::PRESS, Value::Null);
    assert_eq!(h.tree.current_root().unwrap().str_prop(prop::TITLE), Some("Budget"));
}

#[test]
fn utility_converter_and_text_tools() {
    let art = build("utility");
    eprintln!("utility: core {} KiB, component {} KiB", art.core_size / 1024, art.component.len() / 1024);
    let mut h = Harness::start(&art.component);

    assert_eq!(h.tree.current_root().unwrap().str_prop(prop::TITLE), Some("Converter"));

    // Default: 1 meter -> feet. (`style="mono"` is an enum prop, not a
    // string prop, so it cannot be matched with `str_prop`; match on the
    // rendered text instead.)
    let result = h.one_in_current(ControlKind::Text, |n| n.text.as_deref().is_some_and(|t| t.contains(" = ")));
    assert!(h.text_of(result).contains("1 meters = 3.281 feet"), "{}", h.text_of(result));

    // Switch kind to Weight: the length Pickers disappear, weight Pickers
    // appear.
    let kind = h.one_in_current(ControlKind::Picker, |n| n.str_prop(prop::LABEL) == Some("Kind"));
    h.fire(kind, event::CHANGE, "Weight".into());
    assert_eq!(h.find_in_current(ControlKind::Picker, |n| n.str_prop(prop::LABEL) == Some("From")).len(), 1);
    let from = h.one_in_current(ControlKind::Picker, |n| n.str_prop(prop::LABEL) == Some("From"));
    assert_eq!(h.tree.get(from).unwrap().str_prop(prop::VALUE), Some("kilograms"));

    // Switch to the "text" primary screen. Real hosts switch primary
    // screens from their own chrome (a sidebar/tab bar), not a semantic-tree
    // event, so the test drives `Tree::current_screen` directly, the same
    // field the host sets (`main.tsx` declares `text` as screen index 1).
    h.tree.current_screen = 1;
    assert_eq!(h.tree.current_root().unwrap().str_prop(prop::TITLE), Some("Text tools"));

    let input = h.one_in_current(ControlKind::TextArea, |n| n.str_prop(prop::LABEL) == Some("Text"));
    h.fire(input, event::CHANGE, "hello world".into());
    let title_case = h.one_in_current(ControlKind::Button, |n| n.str_prop(prop::LABEL) == Some("Title Case"));
    h.fire(title_case, event::PRESS, Value::Null);
    let output = h.one_in_current(ControlKind::Text, |n| n.text.as_deref() == Some("Hello World"));
    assert_eq!(h.text_of(output), "Hello World");

    let copy = h.one_in_current(ControlKind::Button, |n| n.str_prop(prop::LABEL) == Some("Copy result"));
    h.fire(copy, event::PRESS, Value::Null);

    // Switch to the "time" tab and check the elapsed breakdown renders.
    let tabs = h.one_in_current(ControlKind::Tabs, |_| true);
    h.fire(tabs, event::CHANGE, "time".into());
    let epoch = h.one_in_current(ControlKind::NumberField, |n| n.str_prop(prop::LABEL) == Some("Epoch (ms)"));
    h.fire(epoch, event::CHANGE, Value::Number(90_061_000.0));
    let breakdown = h.one_in_current(ControlKind::Text, |n| n.text.as_deref().is_some_and(|t| t.contains("since the epoch")));
    assert_eq!(h.text_of(breakdown), "1d 1h 1m 1s since the epoch");
}

/// `examples/pong`: a game built from Level 2 primitives and a 16 ms
/// `setInterval` loop. The test drives the timer queue with a fake clock,
/// reads the ball and paddle positions from the spacer boxes that place
/// them, and checks movement, pause, the paddle buttons, bounces and a
/// point. It prints the time of one tick (event, step and commit) in
/// wasmtime; see docs/GAPS.md "Games".
#[test]
fn pong_plays_a_point() {
    use plinth_protocol::color;
    const FIELD_W: i32 = 75;
    const FIELD_H: i32 = 48;
    const PADDLE_H: i32 = 10;
    const BALL: i32 = 2;
    const HALF_W: i32 = 37;
    const NET_W: i32 = 1;

    let art = build("pong");
    eprintln!("pong: app {} B, component {} B", art.app.len(), art.component.len());
    let mut h = Harness::start(&art.component);

    let labelled = |h: &Harness, label: &str| -> NodeId {
        h.find(ControlKind::Box, |n| n.str_prop(prop::LABEL) == Some(label))
            .into_iter()
            .chain(h.find(ControlKind::Pressable, |n| n.str_prop(prop::LABEL) == Some(label)))
            .next()
            .unwrap_or_else(|| panic!("no node labelled {label}"))
    };
    let int = |h: &Harness, id: NodeId, p: u16| h.tree.get(id).unwrap().prop(p).and_then(Value::as_int).unwrap_or(0);
    let spacer_before = |h: &Harness, id: NodeId| -> NodeId {
        let parent = h.tree.get(h.tree.get(id).unwrap().parent).unwrap();
        let i = parent.children.iter().position(|c| *c == id).unwrap();
        parent.children[i - 1]
    };
    let paddle = |h: &Harness, label: &str| {
        let p = labelled(h, label);
        int(h, spacer_before(h, p), prop::HEIGHT)
    };
    let ball = |h: &Harness| -> (i32, i32) {
        for (label, dx) in [("Ball (left half)", 0), ("Ball (right half)", HALF_W + NET_W)] {
            let b = labelled(h, label);
            if h.tree.get(b).unwrap().enum_prop(prop::BG) == color::NONE {
                continue;
            }
            let row = h.tree.get(b).unwrap().parent;
            return (dx + int(h, spacer_before(h, b), prop::WIDTH), int(h, spacer_before(h, row), prop::HEIGHT));
        }
        panic!("no ball is visible");
    };
    let press = |h: &mut Harness, label: &str| {
        let id = labelled(h, label);
        h.fire(id, event::PRESS, Value::Null)
    };
    let score_texts = |h: &Harness| -> Vec<String> {
        h.find(ControlKind::Span, |_| true).into_iter().map(|id| h.text_of(id)).collect()
    };
    let scores = |h: &Harness| -> (u32, u32) {
        let t = score_texts(h);
        let you = t.iter().position(|s| s == "You").unwrap();
        let cpu = t.iter().position(|s| s == "Computer").unwrap();
        (t[you + 1].parse().unwrap(), t[cpu + 1].parse().unwrap())
    };

    assert_eq!(scores(&h), (0, 0));
    assert_eq!(paddle(&h, "Your paddle"), (FIELD_H - PADDLE_H) / 2);
    let start = ball(&h);
    assert!(h.guest.next_timer_deadline().is_none(), "no loop before Start");

    press(&mut h, "Start");
    let t0 = Instant::now();
    let mut now = t0;
    let mut tick = |h: &mut Harness| -> Duration {
        now += Duration::from_millis(16);
        let s = Instant::now();
        h.fire_timers(now);
        s.elapsed()
    };
    for _ in 0..10 {
        tick(&mut h);
    }
    let moved = ball(&h);
    assert!(moved.0 > start.0 && moved.1 != start.1, "the ball moves: {start:?} -> {moved:?}");

    press(&mut h, "Pause");
    assert!(h.guest.next_timer_deadline().is_none(), "Pause stops the loop");
    press(&mut h, "Resume");

    press(&mut h, "Up");
    for _ in 0..5 {
        tick(&mut h);
    }
    assert!(paddle(&h, "Your paddle") < (FIELD_H - PADDLE_H) / 2, "Up moves the paddle up");
    press(&mut h, "Down");
    for _ in 0..40 {
        tick(&mut h);
    }
    assert_eq!(paddle(&h, "Your paddle"), FIELD_H - PADDLE_H, "Down stops at the bottom wall");
    press(&mut h, "Stop");

    // Play until a wall bounce, a return by the computer and a point.
    let (mut wall, mut ret) = (false, false);
    let (mut dx, mut dy) = (0, 0);
    let mut prev = ball(&h);
    let mut times = Vec::new();
    let mut n = 0;
    while (!wall || !ret || scores(&h) == (0, 0)) && n < 3000 && h.guest.next_timer_deadline().is_some() {
        times.push(tick(&mut h));
        n += 1;
        let b = ball(&h);
        let (ndx, ndy) = ((b.0 - prev.0).signum(), (b.1 - prev.1).signum());
        if dy != 0 && ndy != 0 && ndy != dy && (b.1 <= 1 || b.1 >= FIELD_H - BALL - 1) {
            wall = true;
        }
        if dx > 0 && ndx < 0 && prev.0 >= FIELD_W - BALL - 2 && b.0 >= FIELD_W - BALL - 6 {
            ret = true;
        }
        if ndx != 0 {
            dx = ndx;
        }
        if ndy != 0 {
            dy = ndy;
        }
        prev = b;
    }
    assert!(wall, "a wall bounce");
    assert!(ret, "the computer returned the ball");
    assert!(scores(&h).1 >= 1, "the computer scored: {:?}", scores(&h));

    times.sort();
    eprintln!(
        "pong: {} ticks, median {:?}, p95 {:?} per tick (wasmtime, event + commit + tree apply)",
        times.len(),
        times[times.len() / 2],
        times[times.len() * 95 / 100]
    );
}
