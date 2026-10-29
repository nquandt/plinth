//! UI API 1.14 on the desktop: `hidden` (also in a partial style) removes an
//! element from layout, focus and the accessibility tree; `Screen
//! layout="full"` uses the whole window width; `minWidth` and `minHeight`
//! give a floor; `pressed` and `selected` on a `Pressable` go to assistive
//! technology and draw the `selected` token. `web/test/run-ui114.mjs` checks
//! the same facts in the DOM.

use gpui::accesskit::{Node, Role, Toggled, TreeUpdate};
use gpui::{
    AnyWindowHandle, AppContext as _, HeadlessAppContext, Keystroke, MouseButton, MouseDownEvent, MouseUpEvent, Pixels,
    PlatformHeadlessRenderer, PlatformInput, Point, WindowHandle, point, px, size,
};
use plinth_compiler::driver::MemFs;
use plinth_runner_wasmtime::{Guest, Limits, Runner};
use plinth_ui::{GuestPort, PlinthRoot};

struct WasmGuest {
    guest: Guest,
    _runner: Runner,
}

impl GuestPort for WasmGuest {
    fn dispatch(&mut self, events: &[u8]) -> anyhow::Result<Vec<Vec<u8>>> {
        self.guest.on_event(events)
    }
}

/// The app: three pressables in a stack, two of them hidden in some width
/// class; the layout of the screen is a parameter.
fn app_source(layout: &str) -> String {
    format!(
        r#"
import {{ app, signal, Screen, Box, Pressable, Span }} from "plinth:ui";

const on = signal(false);
const choice = signal(false);

function Home() {{
  return (
    <Screen title="UI 1.14"{layout}>
      <Box label="Stack" gap={{3}}>
        <Pressable label="First" role="button" onPress={{() => {{}}}} height={{8}} />
        <Pressable label="Not on compact" role="button" onPress={{() => {{}}}} height={{8}} compact={{{{ hidden: true }}}} />
        <Pressable label="Not on wide" role="button" onPress={{() => {{}}}} height={{8}} wide={{{{ hidden: true }}}} />
        <Pressable label="Never there" role="button" onPress={{() => {{}}}} height={{8}} hidden={{true}} />
        <Span hidden={{true}}>Hidden words</Span>
        <Span compact={{{{ hidden: true }}}}>Words off on compact</Span>
        <Pressable label="Last" role="button" onPress={{() => {{}}}} height={{8}} />
      </Box>
      <Box label="Wide box" width="full" height={{6}} />
      <Box label="Floor row" direction="row" width={{100}}>
        <Box label="Floor A" grow={{1}} minWidth={{70}} minHeight={{20}} />
        <Box label="Floor B" grow={{1}} />
      </Box>
      <Pressable label="Chip" role="button" width={{30}} height={{8}} pressed={{on()}} onPress={{() => on.set(!on())}} />
      <Pressable label="Own look" role="button" width={{30}} height={{8}} bg="accent" pressed={{on()}} onPress={{() => on.set(!on())}} />
      <Pressable label="Choice" role="button" width={{30}} height={{8}} selected={{choice()}} onPress={{() => choice.set(!choice())}} />
      <Pressable label="Current page" role="link" width={{30}} height={{8}} selected={{true}} onPress={{() => {{}}}} />
    </Screen>
  );
}}

export default app({{ screens: {{ home: {{ title: "UI 1.14", component: Home }} }} }});
"#
    )
}

struct Harness {
    cx: HeadlessAppContext,
    window: WindowHandle<PlinthRoot>,
    scale: f64,
}

impl Harness {
    fn new(source: &str, width: f32, height: f32) -> Self {
        let fs = MemFs::default().with("app/main.tsx", source);
        let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[]).expect("compile");
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        let artifact = artifact.unwrap_or_else(|| panic!("the fixture has errors:\n{}", diags.join("\n")));
        let platform = gpui_platform::current_platform(true);
        let mut cx = HeadlessAppContext::with_platform(platform.text_system(), std::sync::Arc::new(()), || {
            gpui_wgpu::WgpuHeadlessRenderer::new()
                .map(|r| Box::new(r) as Box<dyn PlatformHeadlessRenderer>)
                .map_err(|e| log::error!("no headless renderer: {e:#}"))
                .ok()
        });
        cx.update(plinth_ui::init);
        let runner = Runner::new().unwrap();
        let mut guest = runner.load(&artifact.component, Limits::default()).unwrap();
        let commits = guest.init(&[]).unwrap();
        let port = Box::new(WasmGuest { guest, _runner: runner });
        let window = cx.open_window(size(px(width), px(height)), move |_, cx| cx.new(|cx| PlinthRoot::new(port, commits, "indigo", cx))).unwrap();
        cx.update(|cx| window.update(cx, |_, window, _| window.set_a11y_forced(true))).unwrap();
        cx.run_until_parked();
        let scale = f64::from(cx.update(|cx| window.update(cx, |_, window, _| window.scale_factor())).unwrap());
        Self { cx, window, scale }
    }

    fn handle(&self) -> AnyWindowHandle {
        self.window.into()
    }

    fn tree(&mut self) -> TreeUpdate {
        let window = self.window;
        self.cx.update(|cx| window.update(cx, |_, window, _| window.a11y_tree().cloned())).unwrap().expect("a11y tree")
    }

    fn node(&mut self, role: Role, name: &str) -> Option<Node> {
        self.tree().nodes.iter().find(|(_, n)| n.role() == role && n.label() == Some(name)).map(|(_, n)| n.clone())
    }

    /// `(x0, y0, x1, y1)` of a node in logical pixels, if it is there.
    fn bounds(&mut self, role: Role, name: &str) -> Option<(f64, f64, f64, f64)> {
        let s = self.scale;
        self.node(role, name).and_then(|n| n.bounds()).map(|b| (b.x0 / s, b.y0 / s, b.x1 / s, b.y1 / s))
    }

    fn must(&mut self, role: Role, name: &str) -> (f64, f64, f64, f64) {
        self.bounds(role, name).unwrap_or_else(|| panic!("no {role:?} {name:?}"))
    }

    fn center(&mut self, role: Role, name: &str) -> Point<Pixels> {
        let (x0, y0, x1, y1) = self.must(role, name);
        point(px(((x0 + x1) / 2.0) as f32), px(((y0 + y1) / 2.0) as f32))
    }

    fn click(&mut self, p: Point<Pixels>) {
        for up in [false, true] {
            let input = if up {
                PlatformInput::MouseUp(MouseUpEvent { position: p, modifiers: Default::default(), button: MouseButton::Left, click_count: 1 })
            } else {
                PlatformInput::MouseDown(MouseDownEvent { position: p, modifiers: Default::default(), button: MouseButton::Left, click_count: 1, first_mouse: false })
            };
            self.cx.update_window(self.handle(), |_, window, cx| window.dispatch_event(input, cx)).unwrap();
            self.cx.run_until_parked();
        }
    }

    fn tab(&mut self) {
        let stroke = Keystroke::parse("tab").unwrap();
        self.cx.update_window(self.handle(), |_, window, cx| window.dispatch_keystroke(stroke, cx)).unwrap();
        self.cx.run_until_parked();
    }

    /// The labels of the focused nodes after each of `n` presses of Tab.
    fn tab_labels(&mut self, n: usize) -> Vec<String> {
        (0..n)
            .map(|_| {
                self.tab();
                let tree = self.tree();
                tree.nodes.iter().find(|(i, _)| *i == tree.focus).and_then(|(_, n)| n.label()).unwrap_or("").to_owned()
            })
            .collect()
    }

    /// The RGBA pixel at a logical point.
    fn pixel(&mut self, x: f64, y: f64) -> [u8; 4] {
        self.cx.update_window(self.handle(), |_, window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        }).unwrap();
        let image = self.cx.capture_screenshot(self.handle()).expect("screenshot");
        image.get_pixel((x * self.scale) as u32, (y * self.scale) as u32).0
    }

    fn shot(&mut self, name: &str) {
        let shots = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/shots");
        std::fs::create_dir_all(&shots).unwrap();
        let image = self.cx.capture_screenshot(self.handle()).expect("screenshot");
        image.save(shots.join(format!("{name}.png"))).expect("write the screenshot");
    }
}

/// `hidden`: no node, no tab stop and no layout space (not even the gap), and
/// the width class decides, live.
#[test]
fn hidden_leaves_layout_focus_and_the_accessibility_tree() {
    let source = app_source("");

    // Compact (400 px): "Not on compact" and "Words off on compact" are gone.
    let mut h = Harness::new(&source, 400., 900.);
    assert!(h.node(Role::Button, "Not on compact").is_none(), "compact: the element is in the tree");
    assert!(h.node(Role::Button, "Never there").is_none(), "an always hidden element is in the tree");
    assert!(h.node(Role::Label, "Hidden words").is_none(), "a hidden Span is in the tree");
    assert!(h.node(Role::Label, "Words off on compact").is_none(), "compact: the Span is in the tree");
    let first = h.must(Role::Button, "First");
    let wide_only = h.must(Role::Button, "Not on wide");
    // The stack gap is 3 units (12 px): the hidden elements leave no gap.
    assert!((wide_only.1 - first.3 - 12.0).abs() < 1.0, "compact: a hidden element left a gap: {first:?} {wide_only:?}");
    let tabs = h.tab_labels(3);
    assert!(tabs.iter().all(|l| l != "Not on compact" && l != "Never there"), "a hidden element is a tab stop: {tabs:?}");
    assert!(tabs.contains(&"Not on wide".to_owned()) && tabs.contains(&"Last".to_owned()), "compact tab stops: {tabs:?}");
    h.shot("ui114-compact");

    // Wide (1300 px): the other pair is gone, the first is back.
    let mut h = Harness::new(&source, 1300., 900.);
    assert!(h.node(Role::Button, "Not on compact").is_some(), "wide: the element is not in the tree");
    assert!(h.node(Role::Button, "Not on wide").is_none(), "wide: the hidden element is in the tree");
    assert!(h.node(Role::Button, "Never there").is_none());
    assert!(h.node(Role::Label, "Words off on compact").is_some(), "wide: the Span is not in the tree");
    let first = h.must(Role::Button, "First");
    let shown = h.must(Role::Button, "Not on compact");
    let last = h.must(Role::Button, "Last");
    assert!((shown.1 - first.3 - 12.0).abs() < 1.0, "wide: the gap above: {first:?} {shown:?}");
    // Between "Not on compact" and "Last" is the Span (the two hidden buttons and the hidden Span leave no gap).
    let words = h.must(Role::Label, "Words off on compact");
    assert!((words.1 - shown.3 - 12.0).abs() < 1.0, "wide: the gap above the Span: {shown:?} {words:?}");
    assert!((last.1 - words.3 - 12.0).abs() < 1.0, "wide: the gap below the Span: {words:?} {last:?}");
    let tabs = h.tab_labels(3);
    assert_eq!(tabs, ["First", "Not on compact", "Last"], "wide tab order");
    h.shot("ui114-wide");
}

/// `Screen layout="full"` uses the whole width; the default keeps the cap.
#[test]
fn screen_layout_full_uses_the_window_width() {
    let mut readable = Harness::new(&app_source(""), 1300., 900.);
    let (x0, _, x1, _) = readable.must(Role::Group, "Wide box");
    assert!(x1 - x0 <= 720.0, "readable: the content is wider than the cap: {}", x1 - x0);
    let mut full = Harness::new(&app_source(" layout=\"full\""), 1300., 900.);
    let (x0, _, x1, _) = full.must(Role::Group, "Wide box");
    // The window is 1300 px; the navigation rail and the padding take some.
    assert!(x1 - x0 > 900.0, "full: the content is not wide: {}", x1 - x0);
    full.shot("ui114-full");
}

/// `minWidth` and `minHeight` are floors in a row that shares its space.
#[test]
fn min_sizes_give_a_floor() {
    let mut h = Harness::new(&app_source(" layout=\"full\""), 1300., 900.);
    let a = h.must(Role::Group, "Floor A");
    let b = h.must(Role::Group, "Floor B");
    // The row is 100 units (400 px). A would get 200 px; the floor is 70 units (280 px).
    assert!(a.2 - a.0 >= 279.0, "minWidth: {a:?}");
    assert!((b.2 - b.0 - 120.0).abs() < 2.0, "B takes the rest: {b:?}");
    assert!(a.3 - a.1 >= 79.0, "minHeight (20 units): {a:?}");
}

/// `pressed` and `selected` reach AccessKit and draw the `selected` token
/// unless the app sets a `bg`.
#[test]
fn pressed_and_selected_are_exposed_and_drawn() {
    let mut h = Harness::new(&app_source(" layout=\"full\""), 1300., 1000.);
    let chip = h.node(Role::Button, "Chip").expect("chip");
    assert_eq!(chip.toggled(), Some(Toggled::False), "pressed={{false}} is a toggle button that is off");
    assert!(h.node(Role::Button, "Choice").unwrap().toggled().is_none(), "no pressed prop: not a toggle");
    assert_eq!(h.node(Role::Button, "Choice").unwrap().is_selected(), Some(false));
    assert_eq!(h.node(Role::Link, "Current page").unwrap().is_selected(), Some(true));

    let (x0, y0, _, y1) = h.must(Role::Button, "Chip");
    let (ox0, oy0, _, oy1) = h.must(Role::Button, "Own look");
    let (cx0, cy0, _, cy1) = h.must(Role::Button, "Choice");
    let chip_off = h.pixel(x0 + 3.0, (y0 + y1) / 2.0);
    let own_off = h.pixel(ox0 + 3.0, (oy0 + oy1) / 2.0);
    let choice_off = h.pixel(cx0 + 3.0, (cy0 + cy1) / 2.0);

    let at = h.center(Role::Button, "Chip");
    h.click(at);
    assert_eq!(h.node(Role::Button, "Chip").unwrap().toggled(), Some(Toggled::True), "pressed after a click");
    assert_eq!(h.node(Role::Button, "Own look").unwrap().toggled(), Some(Toggled::True), "the signal is shared");
    let at = h.center(Role::Button, "Choice");
    h.click(at);
    assert_eq!(h.node(Role::Button, "Choice").unwrap().is_selected(), Some(true), "selected after a click");

    // Move the pointer away (no hover tint), then compare the pixels.
    h.cx.update_window(h.handle(), |_, window, cx| {
        window.dispatch_event(
            PlatformInput::MouseMove(gpui::MouseMoveEvent { position: point(px(5.), px(5.)), pressed_button: None, modifiers: Default::default() }),
            cx,
        )
    })
    .unwrap();
    h.cx.run_until_parked();
    let chip_on = h.pixel(x0 + 3.0, (y0 + y1) / 2.0);
    let own_on = h.pixel(ox0 + 3.0, (oy0 + oy1) / 2.0);
    let choice_on = h.pixel(cx0 + 3.0, (cy0 + cy1) / 2.0);
    assert_ne!(chip_off, chip_on, "a pressed Pressable draws the selected token");
    assert_ne!(choice_off, choice_on, "a selected Pressable draws the selected token");
    assert_eq!(chip_on, choice_on, "pressed and selected draw the same token");
    assert_eq!(own_off, own_on, "an app bg stays");
    h.shot("ui114-pressed");
}
