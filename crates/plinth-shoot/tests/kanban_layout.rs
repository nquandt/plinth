//! `examples/kanban`, a Jira-like board, on the desktop host: a probe of the
//! Level 2 UI model (`docs/KANBAN-FINDINGS.md`). Opens the app with a real
//! `store.sql` database in a temp folder at the three width classes, in
//! light and dark, and checks the layout from the AccessKit tree: four
//! columns side by side on a wide window, one column and a switch on a
//! compact window, a drag of a card to another column, and the frame cost
//! with 500 cards. Writes `target/shots/kanban-*.png` for a visual check.

use gpui::accesskit::{Role, TreeUpdate};
use gpui::{
    AnyWindowHandle, AppContext as _, HeadlessAppContext, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, PlatformHeadlessRenderer, PlatformInput, Point, WindowHandle, point, px, size,
};
use plinth_compiler::driver::DiskFs;
use plinth_package::ProjectConfig;
use plinth_runner_wasmtime::files::{DEFAULT_QUOTA, Files, Owner, private_space_dir};
use plinth_runner_wasmtime::policy::Policy;
use plinth_runner_wasmtime::sql::STORE_SQL;
use plinth_runner_wasmtime::{Guest, Limits, Runner, kv::Kv};
use plinth_ui::{GuestPort, PlinthRoot, SystemSettings};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const ID: &str = "dev.plinth.examples.kanban";

struct WasmGuest {
    guest: Guest,
    _runner: Runner,
}

impl GuestPort for WasmGuest {
    fn dispatch(&mut self, events: &[u8]) -> anyhow::Result<Vec<Vec<u8>>> {
        self.guest.on_event(events)
    }
    fn poll_net_results(&mut self) -> Vec<(u32, plinth_protocol::Value)> {
        self.guest.poll_net_results()
    }
    fn answer_dialog(&mut self, id: u32, value: plinth_protocol::Value) -> anyhow::Result<Vec<Vec<u8>>> {
        self.guest.answer_dialog(id, value)
    }
    fn take_errors(&mut self) -> Vec<String> {
        self.guest.take_errors()
    }
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/kanban")
}

fn compile() -> Vec<u8> {
    let caps = vec![STORE_SQL.to_owned()];
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&DiskFs { root: root() }, &caps).expect("compile");
    let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
    artifact.unwrap_or_else(|| panic!("examples/kanban has errors:\n{}", diags.join("\n"))).component
}

struct Harness {
    cx: HeadlessAppContext,
    window: WindowHandle<PlinthRoot>,
    scale: f64,
    data: PathBuf,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.data);
    }
}

impl Harness {
    fn new(component: &[u8], width: f32, height: f32, dark: bool, tag: &str) -> Self {
        let platform = gpui_platform::current_platform(true);
        let mut cx = HeadlessAppContext::with_platform(platform.text_system(), std::sync::Arc::new(()), || {
            gpui_wgpu::WgpuHeadlessRenderer::new()
                .map(|r| Box::new(r) as Box<dyn PlatformHeadlessRenderer>)
                .map_err(|e| log::error!("no headless renderer: {e:#}"))
                .ok()
        });
        cx.update(plinth_ui::init);
        let settings = SystemSettings { dark: Some(dark), ..SystemSettings::default() };
        cx.update(|cx| plinth_host_desktop::system_settings::apply(settings, cx));
        let data = std::env::temp_dir().join(format!("plinth-kanban-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let files = Files::open(private_space_dir(&data, &Owner::Dev, ID), DEFAULT_QUOTA);
        let runner = Runner::new().unwrap();
        let mut guest = runner
            .load_with_policy(
                component,
                Limits::default(),
                Policy::new([STORE_SQL]),
                Kv::in_memory(),
                Box::new(plinth_runner_wasmtime::MemoryClipboard::default()),
            )
            .unwrap();
        guest.set_files(files);
        let commits = guest.init(&[]).unwrap();
        let port = Box::new(WasmGuest { guest, _runner: runner });
        let text = std::fs::read_to_string(root().join("plinth.toml")).unwrap();
        let theme = ProjectConfig::parse(&text).unwrap().theme.expect("examples/kanban has a [theme]");
        let window = cx
            .open_window(size(px(width), px(height)), move |_, cx| {
                cx.new(|cx| {
                    let mut root = PlinthRoot::new(port, commits, "indigo", cx);
                    assert!(root.set_theme(Some(&theme)), "the theme applies");
                    root
                })
            })
            .unwrap();
        cx.update(|cx| window.update(cx, |_, window, _| window.set_a11y_forced(true))).unwrap();
        cx.run_until_parked();
        let scale = f64::from(cx.update(|cx| window.update(cx, |_, window, _| window.scale_factor())).unwrap());
        let mut h = Self { cx, window, scale, data };
        h.until("the first issues load", |h| h.bounds(Role::Button, "KAN-1: Set up the project").is_some());
        h
    }

    fn handle(&self) -> AnyWindowHandle {
        self.window.into()
    }

    /// Delivers the answers of the database, until `done` is true.
    fn until(&mut self, what: &str, done: impl Fn(&mut Harness) -> bool) {
        let start = Instant::now();
        loop {
            let window = self.window;
            self.cx.update(|cx| window.update(cx, |root, _, cx| root.poll_timers(cx))).unwrap();
            self.cx.run_until_parked();
            if done(self) {
                return;
            }
            assert!(start.elapsed() < Duration::from_secs(30), "timed out: {what}");
            std::thread::sleep(Duration::from_millis(3));
        }
    }

    fn tree(&mut self) -> TreeUpdate {
        let window = self.window;
        self.cx.update(|cx| window.update(cx, |_, window, _| window.a11y_tree().cloned())).unwrap().expect("a11y tree")
    }

    /// `(x0, y0, x1, y1)` of a node in logical pixels.
    fn bounds(&mut self, role: Role, name: &str) -> Option<(f64, f64, f64, f64)> {
        let s = self.scale;
        let tree = self.tree();
        tree.nodes
            .iter()
            .find(|(_, n)| n.role() == role && n.label() == Some(name))
            .and_then(|(_, n)| n.bounds())
            .map(|b| (b.x0 / s, b.y0 / s, b.x1 / s, b.y1 / s))
    }

    /// True when some node has this label or value.
    fn has_text(&mut self, text: &str) -> bool {
        let tree = self.tree();
        tree.nodes.iter().any(|(_, n)| n.label() == Some(text) || n.value() == Some(text))
    }

    fn must(&mut self, role: Role, name: &str) -> (f64, f64, f64, f64) {
        self.bounds(role, name).unwrap_or_else(|| panic!("no {role:?} {name:?}"))
    }

    fn center(&mut self, role: Role, name: &str) -> Point<Pixels> {
        let (x0, y0, x1, y1) = self.must(role, name);
        point(px(((x0 + x1) / 2.0) as f32), px(((y0 + y1) / 2.0) as f32))
    }

    fn input(&mut self, input: PlatformInput) {
        self.cx.update_window(self.handle(), |_, window, cx| window.dispatch_event(input, cx)).unwrap();
        self.cx.run_until_parked();
    }

    fn click(&mut self, p: Point<Pixels>) {
        for up in [false, true] {
            if up {
                self.input(PlatformInput::MouseUp(MouseUpEvent { position: p, modifiers: Default::default(), button: MouseButton::Left, click_count: 1 }));
            } else {
                self.input(PlatformInput::MouseDown(MouseDownEvent { position: p, modifiers: Default::default(), button: MouseButton::Left, click_count: 1, first_mouse: false }));
            }
        }
    }

    fn drag(&mut self, from: Point<Pixels>, to: Point<Pixels>) {
        self.input(PlatformInput::MouseDown(MouseDownEvent { position: from, modifiers: Default::default(), button: MouseButton::Left, click_count: 1, first_mouse: false }));
        for p in [point(from.x + px(6.), from.y + px(3.)), to] {
            self.input(PlatformInput::MouseMove(MouseMoveEvent { position: p, pressed_button: Some(MouseButton::Left), modifiers: Default::default() }));
        }
        self.input(PlatformInput::MouseUp(MouseUpEvent { position: to, modifiers: Default::default(), button: MouseButton::Left, click_count: 1 }));
        self.input(PlatformInput::MouseMove(MouseMoveEvent { position: to, pressed_button: None, modifiers: Default::default() }));
    }

    fn shot(&mut self, name: &str) {
        let shots = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/shots");
        std::fs::create_dir_all(&shots).unwrap();
        self.cx.update_window(self.handle(), |_, window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        }).unwrap();
        let image = self.cx.capture_screenshot(self.handle()).expect("screenshot");
        image.save(shots.join(format!("kanban-{name}.png"))).expect("write the screenshot");
    }

    fn median_draw_ms(&mut self) -> f64 {
        let handle = self.handle();
        let mut times: Vec<f64> = (0..12)
            .map(|_| {
                self.cx.update_window(handle, |_, window, cx| {
                    window.refresh();
                    let t = Instant::now();
                    let arena = window.draw(cx);
                    let ms = t.elapsed().as_secs_f64() * 1000.0;
                    arena.clear(cx);
                    ms
                }).unwrap()
            })
            .collect();
        times.sort_by(f64::total_cmp);
        times[6]
    }
}

const SIZES: [(&str, f32, f32); 3] = [("compact", 400., 760.), ("regular", 900., 760.), ("wide", 1280., 800.)];

/// The board at each width class, in light and dark: shots, and the layout.
#[test]
fn the_board_adapts_to_the_width_class() {
    let component = compile();
    for (class, w, h) in SIZES {
        for dark in [false, true] {
            let mode = if dark { "dark" } else { "light" };
            let mut app = Harness::new(&component, w, h, dark, &format!("{class}-{mode}"));
            app.shot(&format!("{class}-{mode}"));
            let backlog = app.must(Role::ScrollView, "Backlog column");
            let review = app.must(Role::ScrollView, "Review column");
            match class {
                "compact" => {
                    // One column shows (the first); the others have no height. The switch shows.
                    assert!(backlog.3 - backlog.1 > 100.0, "compact: the Backlog column shows");
                    assert!(review.3 - review.1 < 2.0, "compact: the Review column is folded away: {review:?}");
                    let switch = app.must(Role::Button, "Show Review");
                    assert!(switch.3 > switch.1, "compact: the switch shows");
                    let at = app.center(Role::Button, "Show Review");
                    app.click(at);
                    let review = app.must(Role::ScrollView, "Review column");
                    assert!(review.3 - review.1 > 100.0, "compact: Review shows after the switch");
                    app.shot(&format!("{class}-{mode}-review"));
                }
                _ => {
                    // The columns are side by side, on the same row; the switch has no height.
                    assert!(review.0 > backlog.0 + 100.0, "{class}: Review is right of Backlog");
                    assert!((review.1 - backlog.1).abs() < 2.0, "{class}: the columns share a top");
                    let switch = app.bounds(Role::Button, "Show Review");
                    assert!(switch.is_none_or(|b| b.3 - b.1 < 2.0), "{class}: the switch has no height: {switch:?}");
                }
            }
        }
    }
}

/// A drag of a card onto another column moves it, and the move is saved.
#[test]
fn a_drag_moves_a_card_to_another_column() {
    let component = compile();
    let mut app = Harness::new(&component, 1280., 800., false, "drag");
    let card = app.center(Role::Button, "KAN-7: Export issues to CSV");
    let done = app.must(Role::ScrollView, "Done column");
    let target = point(px(((done.0 + done.2) / 2.0) as f32), px((done.1 + 150.0) as f32));
    app.drag(card, target);
    app.until("the card is in Done", |h| h.bounds(Role::Button, "KAN-7: Export issues to CSV").is_some_and(|b| b.0 >= done.0 - 1.0));
    app.shot("after-drag");
    let moved = app.must(Role::Button, "KAN-7: Export issues to CSV");
    assert!(moved.0 >= done.0 && moved.2 <= done.2 + 1.0, "the card is inside the Done column: {moved:?} {done:?}");
}

/// 500 more cards: the first frame and the cost of a frame (debug build).
#[test]
fn five_hundred_cards_draw_in_a_usable_frame() {
    let component = compile();
    let mut app = Harness::new(&component, 1280., 800., false, "many");
    let before = app.median_draw_ms();
    let add = app.center(Role::Button, "Add 500 demo issues");
    app.click(add);
    app.until("500 more issues", |h| h.has_text("510 issues, 2 done"));
    let after = app.median_draw_ms();
    app.shot("many");
    eprintln!("kanban frame: 10 cards {before:.1} ms, 510 cards {after:.1} ms (debug build, 1280 x 800)");
    assert!(after < 400.0, "510 cards: median frame {after:.1} ms");
}
