//! PRODUCTION U6, part 2 (UI API 1.13) on the desktop: drag and drop in one
//! app (`dragValue`, `onDrop`, `List.onReorder`), the keyboard path
//! (Alt+Up and Alt+Down on the focused row), the drop highlight, a modal
//! layer that stops a drag, and a `Dialog` with a small form (GAPS 7G-7).
//! `web/test/run-a11y.mjs` (`checkDragDrop`) checks the same facts in the
//! browser.

use gpui::accesskit::{Node, Role, TreeUpdate};
use gpui::{
    AnyWindowHandle, AppContext as _, HeadlessAppContext, Keystroke, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, PlatformHeadlessRenderer, PlatformInput, Point,
    WindowHandle, point, px, size,
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

const APP: &str = r#"
import { app, signal, Screen, Text, Button, Box, Scroll, Dialog, Action, List, Row, Slider, TextField } from "plinth:ui";

const items = signal(["Apple", "Banana", "Cherry"]);
const log = signal("none");
const dialog = signal(false);
const size = signal(10);
const name = signal("");

function move(from: number, to: number) {
  const all = items();
  const rest: string[] = [];
  for (let i = 0; i < all.length; i++) {
    if (i !== from) rest.push(all[i]);
  }
  const out: string[] = [];
  for (let i = 0; i < rest.length; i++) {
    if (i === to) out.push(all[from]);
    out.push(rest[i]);
  }
  if (to >= rest.length) out.push(all[from]);
  items.set(out);
  log.set(`reorder ${from} ${to}`);
}

function Home() {
  return (
    <Screen title="Drag">
      <Text>{`log: ${log()}`}</Text>
      <Text>{`order: ${items().join(",")}`}</Text>
      <Text>{`size: ${size()}`}</Text>
      <List items={items()} key={(s) => s} row={(s) => <Row title={s} dragValue={s} />} onReorder={move} />
      <Box label="Basket" width={60} height={20} bg="surface" border="border"
           onDrop={(v, x, y) => log.set(`drop ${v} ${x > 0 && x < 60} ${y > 0 && y < 20}`)} />
      <Scroll label="Strip" direction="row" width={60} height={14} bg="surface.alt">
        <Box width={60} height={10} />
        <Box width={60} height={10} />
        <Box label="Far basket" width={30} height={10} bg="surface" border="border"
             onDrop={(v) => log.set(`far drop ${v}`)} />
      </Scroll>
      <Box label="Card" dragValue="card" width={20} height={8} bg="surface.alt" />
      <Button label="Open dialog" onPress={() => dialog.set(true)} />
      <Dialog open={dialog} title="Size" message="Pick a size." onClose={() => log.set("closed")}
              actions={[<Action label="OK" role="primary" onPress={() => dialog.set(false)} />]}>
        <Slider label="Size" value={size} min={0} max={100} step={1} />
        <TextField label="Name" value={name} />
      </Dialog>
    </Screen>
  );
}

export default app({ screens: { home: { title: "Drag", component: Home } } });
"#;

struct Harness {
    cx: HeadlessAppContext,
    window: WindowHandle<PlinthRoot>,
    scale: f64,
}

impl Harness {
    fn new() -> Self {
        let fs = MemFs::default().with("app/main.tsx", APP);
        let (front, artifact) =
            plinth_compiler::compile_with_capabilities(&fs, &[]).expect("compile");
        let diags: Vec<String> = front
            .diags
            .iter()
            .map(|d| front.sources.render(d))
            .collect();
        let artifact =
            artifact.unwrap_or_else(|| panic!("the fixture has errors:\n{}", diags.join("\n")));
        let platform = gpui_platform::current_platform(true);
        let mut cx = HeadlessAppContext::with_platform(
            platform.text_system(),
            std::sync::Arc::new(()),
            || {
                gpui_wgpu::WgpuHeadlessRenderer::new()
                    .map(|r| Box::new(r) as Box<dyn PlatformHeadlessRenderer>)
                    .map_err(|e| log::error!("no headless renderer: {e:#}"))
                    .ok()
            },
        );
        cx.update(plinth_ui::init);
        let runner = Runner::new().unwrap();
        let mut guest = runner.load(&artifact.component, Limits::default()).unwrap();
        let commits = guest.init(&[]).unwrap();
        let port = Box::new(WasmGuest {
            guest,
            _runner: runner,
        });
        let window = cx
            .open_window(size(px(800.), px(700.)), move |_, cx| {
                cx.new(|cx| PlinthRoot::new(port, commits, "indigo", cx))
            })
            .unwrap();
        cx.update(|cx| window.update(cx, |_, window, _| window.set_a11y_forced(true)))
            .unwrap();
        cx.run_until_parked();
        let scale = f64::from(
            cx.update(|cx| window.update(cx, |_, window, _| window.scale_factor()))
                .unwrap(),
        );
        Self { cx, window, scale }
    }

    fn handle(&self) -> AnyWindowHandle {
        self.window.into()
    }

    fn tree(&mut self) -> TreeUpdate {
        let window = self.window;
        self.cx
            .update(|cx| window.update(cx, |_, window, _| window.a11y_tree().cloned()))
            .unwrap()
            .expect("a11y tree")
    }

    /// `(x0, y0, x1, y1)` of a node in logical pixels, if it is there.
    fn bounds(&mut self, role: Role, name: &str) -> Option<(f64, f64, f64, f64)> {
        let s = self.scale;
        let tree = self.tree();
        tree.nodes
            .iter()
            .find(|(_, n)| n.role() == role && n.label() == Some(name))
            .and_then(|(_, n)| n.bounds())
            .map(|b| (b.x0 / s, b.y0 / s, b.x1 / s, b.y1 / s))
    }

    fn must_bounds(&mut self, role: Role, name: &str) -> (f64, f64, f64, f64) {
        self.bounds(role, name)
            .unwrap_or_else(|| panic!("no {role:?} {name:?}"))
    }

    fn center(&mut self, role: Role, name: &str) -> Point<Pixels> {
        let (x0, y0, x1, y1) = self.must_bounds(role, name);
        point(px(((x0 + x1) / 2.0) as f32), px(((y0 + y1) / 2.0) as f32))
    }

    fn input(&mut self, input: PlatformInput) {
        self.cx
            .update_window(self.handle(), |_, window, cx| {
                window.dispatch_event(input, cx)
            })
            .unwrap();
        self.cx.run_until_parked();
    }

    /// Presses the main button at `from` and moves to `to` (in two steps).
    /// The button stays down: `release` ends the drag.
    fn drag_to(&mut self, from: Point<Pixels>, to: Point<Pixels>) {
        self.input(PlatformInput::MouseDown(MouseDownEvent {
            position: from,
            modifiers: Default::default(),
            button: MouseButton::Left,
            click_count: 1,
            first_mouse: false,
        }));
        let near = point(from.x + px(6.), from.y + px(3.));
        for p in [near, to] {
            self.input(PlatformInput::MouseMove(MouseMoveEvent {
                position: p,
                pressed_button: Some(MouseButton::Left),
                modifiers: Default::default(),
            }));
        }
    }

    fn release(&mut self, at: Point<Pixels>) {
        self.input(PlatformInput::MouseUp(MouseUpEvent {
            position: at,
            modifiers: Default::default(),
            button: MouseButton::Left,
            click_count: 1,
        }));
        self.input(PlatformInput::MouseMove(MouseMoveEvent {
            position: at,
            pressed_button: None,
            modifiers: Default::default(),
        }));
    }

    fn click(&mut self, p: Point<Pixels>) {
        self.input(PlatformInput::MouseDown(MouseDownEvent {
            position: p,
            modifiers: Default::default(),
            button: MouseButton::Left,
            click_count: 1,
            first_mouse: false,
        }));
        self.input(PlatformInput::MouseUp(MouseUpEvent {
            position: p,
            modifiers: Default::default(),
            button: MouseButton::Left,
            click_count: 1,
        }));
    }

    fn key(&mut self, keys: &str) {
        for k in keys.split(' ') {
            let stroke = Keystroke::parse(k).unwrap();
            self.cx
                .update_window(self.handle(), |_, window, cx| {
                    window.dispatch_keystroke(stroke, cx)
                })
                .unwrap();
            self.cx.run_until_parked();
        }
    }

    fn text(&mut self, prefix: &str) -> String {
        let tree = self.tree();
        tree.nodes
            .iter()
            .filter_map(|(_, n)| n.label().or(n.value()))
            .find(|l| l.starts_with(prefix))
            .unwrap_or("")
            .to_owned()
    }

    fn log(&mut self) -> String {
        self.text("log: ")
    }

    fn order(&mut self) -> String {
        self.text("order: ")
    }

    /// The message of the polite live region (the drag-and-drop messages).
    fn announced(&mut self) -> String {
        let tree = self.tree();
        tree.nodes
            .iter()
            .find(|(_, n)| n.role() == Role::Status)
            .and_then(|(_, n)| n.label())
            .unwrap_or("")
            .to_owned()
    }

    /// The focused node as `"<role> <name>"`.
    fn now(&mut self) -> String {
        let tree = self.tree();
        let n: Option<&Node> = tree
            .nodes
            .iter()
            .find(|(i, _)| *i == tree.focus)
            .map(|(_, n)| n);
        n.map(|n| format!("{:?} {}", n.role(), n.label().unwrap_or("")))
            .unwrap_or_else(|| "nothing".into())
    }

    fn tab_to(&mut self, want: &str) {
        let mut seen = Vec::new();
        for _ in 0..30 {
            if self.now() == want {
                return;
            }
            self.key("tab");
            seen.push(self.now());
        }
        panic!("Tab never reached {want:?}; it went through {seen:#?}");
    }

    /// Saves the window to `target/shots/<name>.png`, to look at.
    fn shot(&mut self, name: &str) {
        let shots = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/shots");
        std::fs::create_dir_all(&shots).unwrap();
        let image = self
            .cx
            .capture_screenshot(self.handle())
            .expect("screenshot");
        image
            .save(shots.join(format!("{name}.png")))
            .expect("write the screenshot");
    }

    /// The RGBA pixel at a logical point.
    fn pixel(&mut self, x: f64, y: f64) -> [u8; 4] {
        let image = self
            .cx
            .capture_screenshot(self.handle())
            .expect("screenshot");
        let p = image.get_pixel((x * self.scale) as u32, (y * self.scale) as u32);
        p.0
    }
}

/// A drag of a row onto another row of a `List` with `onReorder` sends
/// `[from, to]`; the app moves the item. The drop line shows under the
/// target row while the drag is over it, and the live region tells the
/// result.
#[test]
fn a_drag_reorders_the_list() {
    let mut h = Harness::new();
    assert_eq!(h.order(), "order: Apple,Banana,Cherry");
    let apple = h.center(Role::ListItem, "Apple");
    let cherry = h.center(Role::ListItem, "Cherry");
    let (x0, _, _, y1) = h.must_bounds(Role::ListItem, "Cherry");
    // gpui paints a drag-over border inside the bounds of the element.
    let below = h.pixel(x0 + 30.0, y1 - 1.0);
    h.drag_to(apple, cherry);
    assert_eq!(h.announced(), "Dragging Apple");
    let line = h.pixel(x0 + 30.0, y1 - 1.0);
    assert_ne!(line, below, "the drop line shows under the last row");
    h.shot("drag-reorder");
    h.release(cherry);
    assert_eq!(h.log(), "log: reorder 0 2");
    assert_eq!(h.order(), "order: Banana,Cherry,Apple");
    assert_eq!(h.announced(), "Moved Apple to position 3 of 3");

    // From below: Apple (now last) onto Banana (the first row).
    let apple = h.center(Role::ListItem, "Apple");
    let banana = h.center(Role::ListItem, "Banana");
    h.drag_to(apple, banana);
    h.release(banana);
    assert_eq!(h.log(), "log: reorder 2 0");
    assert_eq!(h.order(), "order: Apple,Banana,Cherry");

    // A drop on the same row, or outside the list, does nothing.
    let banana = h.center(Role::ListItem, "Banana");
    h.drag_to(banana, point(banana.x + px(20.), banana.y));
    h.release(point(banana.x + px(20.), banana.y));
    assert_eq!(h.order(), "order: Apple,Banana,Cherry");
    assert_eq!(h.log(), "log: reorder 2 0");
}

/// Auto-scroll (finding H3): a drag held at the right edge of a horizontal
/// `Scroll` scrolls it, so a drop target that was off screen can take the
/// drop. A drag in the middle of the Scroll does not scroll it.
#[test]
fn a_drag_at_the_edge_of_a_scroll_scrolls_it() {
    let mut h = Harness::new();
    let (sx0, sy0, sx1, sy1) = h.must_bounds(Role::ScrollView, "Strip");
    let (fx0, _, _, _) = h.must_bounds(Role::Group, "Far basket");
    assert!(fx0 > sx1, "the far basket starts off screen: {fx0} vs {sx1}");
    let (cx0, cy0, _, _) = h.must_bounds(Role::Group, "Card");
    let card = point(px((cx0 + 2.0) as f32), px((cy0 + 2.0) as f32));
    let mid_y = ((sy0 + sy1) / 2.0) as f32;

    // The middle of the strip: no scroll.
    let middle = point(px(((sx0 + sx1) / 2.0) as f32), px(mid_y));
    h.drag_to(card, middle);
    for _ in 0..8 {
        std::thread::sleep(std::time::Duration::from_millis(30));
        h.input(PlatformInput::MouseMove(MouseMoveEvent {
            position: middle,
            pressed_button: Some(MouseButton::Left),
            modifiers: Default::default(),
        }));
    }
    let (still, _, _, _) = h.must_bounds(Role::Group, "Far basket");
    assert!((still - fx0).abs() < 1.0, "the strip scrolled in the middle: {still} vs {fx0}");
    h.release(middle);

    // The right edge: it scrolls, though the pointer rests.
    let edge = point(px((sx1 - 4.0) as f32), px(mid_y));
    h.drag_to(card, edge);
    let mut scrolled = false;
    for _ in 0..120 {
        std::thread::sleep(std::time::Duration::from_millis(16));
        h.input(PlatformInput::MouseMove(MouseMoveEvent {
            position: edge,
            pressed_button: Some(MouseButton::Left),
            modifiers: Default::default(),
        }));
        if h.bounds(Role::Group, "Far basket").is_some_and(|b| b.2 <= sx1 + 1.0) {
            scrolled = true;
            break;
        }
    }
    assert!(scrolled, "a drag at the edge did not scroll the strip to the far basket");
    h.shot("drag-autoscroll");
    h.release(edge);
    assert_eq!(h.log(), "log: far drop card");
    // After the drop the strip stays where it is.
    let (a, _, _, _) = h.must_bounds(Role::Group, "Far basket");
    std::thread::sleep(std::time::Duration::from_millis(100));
    h.input(PlatformInput::MouseMove(MouseMoveEvent {
        position: edge,
        pressed_button: None,
        modifiers: Default::default(),
    }));
    let (b, _, _, _) = h.must_bounds(Role::Group, "Far basket");
    assert!((a - b).abs() < 0.5, "the strip scrolls on after the drop");
}

/// An element with `dragValue` dropped on an element with `onDrop` sends
/// `[value, x, y]` (spacing units in the target). The target shows that it
/// takes the drop while the drag is over it.
#[test]
fn a_drop_on_a_target_sends_the_value() {
    let mut h = Harness::new();
    let (bx0, by0, bx1, by1) = h.must_bounds(Role::Group, "Basket");
    let basket = point(
        px(((bx0 + bx1) / 2.0) as f32),
        px(((by0 + by1) / 2.0) as f32),
    );
    let (cx0, cy0, _, _) = h.must_bounds(Role::Group, "Card");
    let card = point(px((cx0 + 2.0) as f32), px((cy0 + 2.0) as f32));
    // A pixel in the basket, away from the card that moves with the pointer.
    let probe = (bx0 + 4.0, by0 + 4.0);
    let before = h.pixel(probe.0, probe.1);
    h.drag_to(card, basket);
    let over = h.pixel(probe.0, probe.1);
    assert_ne!(over, before, "the drop target shows the drag over it");
    h.shot("drag-drop-target");
    h.release(basket);
    assert_eq!(h.log(), "log: drop card true true");
    assert_eq!(h.announced(), "Dropped Card");
    assert_eq!(
        h.pixel(probe.0, probe.1),
        before,
        "the highlight goes after the drop"
    );

    // A row of the list has a drag value too.
    let cherry = h.center(Role::ListItem, "Cherry");
    h.drag_to(cherry, basket);
    h.release(basket);
    assert_eq!(h.log(), "log: drop Cherry true true");
    assert_eq!(
        h.order(),
        "order: Apple,Banana,Cherry",
        "a drop on a target is not a move"
    );
}

/// The keyboard path: a row of a list that the user can reorder is a tab
/// stop; Alt+Up and Alt+Down move it, and the focus stays on it.
#[test]
fn alt_up_and_down_move_the_focused_row() {
    let mut h = Harness::new();
    h.tab_to("ListItem Banana");
    h.key("alt-up");
    assert_eq!(h.log(), "log: reorder 1 0");
    assert_eq!(h.order(), "order: Banana,Apple,Cherry");
    assert_eq!(
        h.now(),
        "ListItem Banana",
        "the focus stays on the moved row"
    );
    assert_eq!(h.announced(), "Moved Banana to position 1 of 3");
    // At the top, Alt+Up does nothing.
    h.key("alt-up");
    assert_eq!(h.log(), "log: reorder 1 0");
    h.key("alt-down alt-down");
    assert_eq!(h.order(), "order: Apple,Cherry,Banana");
    assert_eq!(h.log(), "log: reorder 1 2");
    h.key("alt-down");
    assert_eq!(
        h.order(),
        "order: Apple,Cherry,Banana",
        "at the bottom, Alt+Down does nothing"
    );
    assert_eq!(h.now(), "ListItem Banana");
}

/// GAPS G4: while a `Dialog` is open, a drag of an element under it does
/// not start, and nothing under it takes a drop.
#[test]
fn a_modal_dialog_stops_a_drag() {
    let mut h = Harness::new();
    let basket = h.center(Role::Group, "Basket");
    let (cx0, cy0, _, _) = h.must_bounds(Role::Group, "Card");
    let card = point(px((cx0 + 2.0) as f32), px((cy0 + 2.0) as f32));
    let apple = h.center(Role::ListItem, "Apple");
    let cherry = h.center(Role::ListItem, "Cherry");
    let open = h.center(Role::Button, "Open dialog");
    h.click(open);
    assert!(
        h.bounds(Role::Dialog, "Size").is_some(),
        "the dialog is open"
    );
    h.drag_to(card, basket);
    h.release(basket);
    // The press and the release were on the backdrop: a click that closes
    // the dialog. No drop.
    assert_eq!(h.log(), "log: closed", "no drop under the dialog");
    h.click(open);
    assert!(
        h.bounds(Role::Dialog, "Size").is_some(),
        "the dialog is open again"
    );
    h.drag_to(apple, cherry);
    h.release(cherry);
    assert_eq!(h.log(), "log: closed", "no reorder under the dialog");
    assert_eq!(h.order(), "order: Apple,Banana,Cherry");
    assert_eq!(h.announced(), "", "no drag started");
}

/// GAPS 7G-7: a `Dialog` holds a small form between its message and its
/// actions. The first field gets the focus; the slider takes the arrow
/// keys; Enter in the text field does not close the dialog; Escape closes
/// it and `onClose` runs.
#[test]
fn a_dialog_holds_a_form() {
    let mut h = Harness::new();
    let open = h.center(Role::Button, "Open dialog");
    h.click(open);
    let (_, dy0, _, dy1) = h.must_bounds(Role::Dialog, "Size");
    let (_, sy0, _, sy1) = h.must_bounds(Role::Slider, "Size");
    assert!(sy0 >= dy0 && sy1 <= dy1, "the slider is in the dialog");
    let (_, oy0, _, _) = h.must_bounds(Role::Button, "OK");
    assert!(sy1 <= oy0, "the form is above the actions");
    assert_eq!(h.now(), "Slider Size", "the first field has the focus");
    h.key("right right");
    assert_eq!(h.text("size: "), "size: 12");
    h.key("tab");
    assert!(
        h.now().starts_with("TextInput"),
        "Tab goes to the text field, not {}",
        h.now()
    );
    h.key("enter");
    assert!(
        h.bounds(Role::Dialog, "Size").is_some(),
        "Enter in a field does not close the dialog"
    );
    h.key("escape");
    assert!(h.bounds(Role::Dialog, "Size").is_none(), "Escape closes it");
    assert_eq!(h.log(), "log: closed", "onClose runs");
    assert_eq!(h.now(), "Button Open dialog", "the focus goes back");
}
