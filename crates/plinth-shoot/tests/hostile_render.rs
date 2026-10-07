//! A hostile guest, the renderer part: random trees of real controls with
//! wrong prop types, `NaN`, huge numbers, unknown enums, bad canvas shapes
//! and very long text, then random changes to them. The desktop renderer
//! must draw each one without a panic (`crates/plinth-ui/tests/hostile_ops.rs`
//! checks the tree applier). The random source is fixed;
//! `PLINTH_FUZZ_ITERS` sets the number of windows (default 20).

use gpui::{AppContext as _, HeadlessAppContext, PlatformHeadlessRenderer, px, size};
use plinth_protocol::{NodeId, Op, Value, Writer, nav_kind};
use plinth_ui::{GuestPort, PlinthRoot};
use std::collections::VecDeque;
use std::time::Instant;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }
    fn pick<T: Clone>(&mut self, xs: &[T]) -> T {
        xs[self.below(xs.len() as u64) as usize].clone()
    }
    fn shapes(&mut self) -> String {
        let fields = ["r", "c", "l", "t", "R", "C", "x", "", "1e309", "NaN", "-5", "0", "400", "accent", "nope", "text.muted", "\u{1F600}"];
        (0..self.below(8))
            .map(|_| (0..1 + self.below(8)).map(|_| self.pick(&fields)).collect::<Vec<_>>().join("\u{1F}"))
            .collect::<Vec<_>>()
            .join("\u{1E}")
    }
    fn value(&mut self, depth: u32) -> Value {
        match self.below(if depth > 1 { 9 } else { 10 }) {
            0 => Value::Null,
            1 => Value::Bool(self.below(2) == 1),
            2 => Value::Int(self.pick(&[0, 1, -1, 7, 1000, i32::MAX, i32::MIN])),
            3 => Value::Number(self.pick(&[0.0, -1.0, 0.5, 3.25, 1e308, -1e308, f64::NAN, f64::INFINITY, f64::NEG_INFINITY])),
            4 => Value::Str(self.pick(&["", "a", "Hello", "x\u{0}y", "\u{1F600}\u{200D}", "1/2", "full", "accent", "#zz", "\n\n"]).to_owned()),
            5 => Value::Str("long text ".repeat(self.pick(&[50, 2000]))),
            6 => Value::Enum(self.below(40) as u16),
            7 => Value::Handle(self.next() as u32),
            8 => Value::Str(self.shapes()),
            _ => Value::List((0..self.below(6)).map(|_| self.value(depth + 1)).collect()),
        }
    }
}

/// A commit that builds screen `screen` from scratch: ids from `base`.
fn build_screen(rng: &mut Rng, screen: u32, base: NodeId) -> (Vec<u8>, Vec<NodeId>) {
    let mut w = Writer::new();
    let n = 2 + rng.below(60) as NodeId;
    let root = base;
    w.op(&Op::Create { id: root, kind: 1 });
    w.op(&Op::SetProp { id: root, prop: 1, value: Value::Str(format!("Screen {screen}")) });
    let mut ids = vec![root];
    // Controls that show their children: screen, section, list, group,
    // tabs, sheet, dialog, menu, grid, box, pressable, scroll.
    const CONTAINERS: [u16; 12] = [1, 2, 8, 11, 20, 21, 22, 23, 24, 30, 32, 33];
    let mut containers = vec![root];
    for k in 1..n {
        let id = base + k;
        // Mostly real controls (1..=34), sometimes an unknown kind.
        let kind = if rng.below(15) == 0 { 200 + rng.below(9) as u16 } else { 1 + rng.below(34) as u16 };
        w.op(&Op::Create { id, kind });
        // A tree (no cycle): the parent is a node made before, mostly one
        // that shows its children.
        let parent = if rng.below(5) == 0 { rng.pick(&ids) } else { rng.pick(&containers) };
        if CONTAINERS.contains(&kind) {
            containers.push(id);
        }
        w.op(&Op::Insert { parent, id, before: 0 });
        for _ in 0..rng.below(8) {
            w.op(&Op::SetProp { id, prop: 1 + rng.below(80) as u16, value: rng.value(0) });
        }
        if rng.below(3) == 0 {
            w.op(&Op::Text { id, value: rng.value(1) });
        }
        if rng.below(3) == 0 {
            w.op(&Op::Listen { id, event: 1 + rng.below(11) as u16, handler: rng.next() as u32 });
        }
        ids.push(id);
    }
    w.op(&Op::SetRoot { screen, id: root });
    if rng.below(2) == 0 {
        w.op(&Op::Navigate { kind: nav_kind::MARK_PRIMARY, screen, args: Value::Null });
    }
    (w.as_bytes().to_vec(), ids)
}

/// A later commit: prop changes, moves and removes on live nodes.
fn change(rng: &mut Rng, ids: &[NodeId]) -> Vec<u8> {
    let mut w = Writer::new();
    for _ in 0..1 + rng.below(12) {
        let id = rng.pick(ids);
        match rng.below(6) {
            0 | 1 => w.op(&Op::SetProp { id, prop: 1 + rng.below(80) as u16, value: rng.value(0) }),
            2 => w.op(&Op::Text { id, value: rng.value(1) }),
            3 => w.op(&Op::Move { parent: rng.pick(ids), id, before: rng.pick(ids) }),
            4 => w.op(&Op::Remove { id }),
            _ => w.op(&Op::Navigate { kind: rng.pick(&[nav_kind::PUSH, nav_kind::BACK, nav_kind::SELECT_PRIMARY]), screen: rng.below(3) as u32, args: Value::Null }),
        }
    }
    w.as_bytes().to_vec()
}

/// A guest that answers each timer poll with the next queued commit.
struct Scripted {
    queue: VecDeque<Vec<u8>>,
}

impl GuestPort for Scripted {
    fn dispatch(&mut self, _events: &[u8]) -> anyhow::Result<Vec<Vec<u8>>> {
        Ok(Vec::new())
    }
    fn next_timer_deadline(&self) -> Option<Instant> {
        (!self.queue.is_empty()).then(Instant::now)
    }
    fn fire_due_timers(&mut self, _now: Instant) -> anyhow::Result<Vec<Vec<u8>>> {
        Ok(self.queue.pop_front().into_iter().collect())
    }
}

#[test]
fn hostile_trees_render_without_a_panic() {
    let iters: u64 = std::env::var("PLINTH_FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(20);
    let platform = gpui_platform::current_platform(true);
    let mut cx = HeadlessAppContext::with_platform(platform.text_system(), std::sync::Arc::new(()), || {
        gpui_wgpu::WgpuHeadlessRenderer::new()
            .map(|r| Box::new(r) as Box<dyn PlatformHeadlessRenderer>)
            .map_err(|e| log::error!("no headless renderer: {e:#}"))
            .ok()
    });
    cx.update(plinth_ui::init);

    for case in 0..iters {
        let mut rng = Rng(0xD5A1 + case);
        let mut first = Vec::new();
        let mut ids = Vec::new();
        for screen in 0..1 + rng.below(3) as u32 {
            let (bytes, more) = build_screen(&mut rng, screen, 1 + screen * 100);
            first.push(bytes);
            ids.extend(more);
        }
        let queue: VecDeque<Vec<u8>> = (0..rng.below(6)).map(|_| change(&mut rng, &ids)).collect();
        let steps = queue.len();
        let port = Box::new(Scripted { queue });
        // The width class changes the layout: compact, regular and wide.
        let width = rng.pick(&[360.0, 800.0, 1300.0]);
        eprintln!("case {case}: {} screen(s), {steps} later commit(s), width {width}", first.len());
        let window = cx
            .open_window(size(px(width), px(700.)), move |_, cx| cx.new(|cx| PlinthRoot::new(port, first, "teal", cx)))
            .unwrap();
        cx.update(|cx| window.update(cx, |_, window, _| window.set_a11y_forced(true))).unwrap();
        let t = Instant::now();
        cx.run_until_parked();
        // The window drew: the accessibility tree has the screen's nodes.
        let drawn = cx.update(|cx| window.update(cx, |_, window, _| window.a11y_tree().map_or(0, |t| t.nodes.len()))).unwrap();
        assert!(drawn > 1, "case {case}: nothing drawn");
        eprintln!("  first frame {:?}, {drawn} accessibility nodes", t.elapsed());
        for _ in 0..steps {
            cx.update(|cx| window.update(cx, |root, _, cx| root.poll_timers(cx))).unwrap();
            cx.run_until_parked();
        }
        for screen in 0..3 {
            cx.update(|cx| window.update(cx, |root, _, cx| root.select_screen(screen, cx))).unwrap();
            cx.run_until_parked();
        }
        cx.update(|cx| window.update(cx, |_, window, _| window.remove_window())).unwrap();
        cx.run_until_parked();
    }
}
