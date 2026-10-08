//! PRODUCTION U6, part 1 (UI API 1.13) on the desktop: `onContextMenu` with
//! a secondary click and with Shift+F10, a `Menu` that the app opens at the
//! point, and a modal layer that blocks the pointer to the elements under
//! it (GAPS G4). `web/test/run-a11y.mjs` (`checkContextMenu`) checks the
//! same facts in the browser.

use gpui::accesskit::{Node, Role, TreeUpdate};
use gpui::{
    AnyWindowHandle, AppContext as _, HeadlessAppContext, Keystroke, MouseButton, MouseDownEvent, MouseUpEvent, PlatformHeadlessRenderer,
    PlatformInput, Pixels, Point, WindowHandle, point, px, size,
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
import { app, signal, Screen, Text, Button, Box, Dialog, Menu, Action, List, Row } from "plinth:ui";

const log = signal("none");
const count = signal(0);
const menu = signal(false);
const dialog = signal(false);

function Home() {
  return (
    <Screen title="Context">
      <Text>{`log: ${log()}`}</Text>
      <Text>{`count: ${count()}`}</Text>
      <Box label="Pad" width={50} height={25} bg="surface"
           onPointerDown={(x, y) => log.set(`pad down ${x} ${y}`)}
           onContextMenu={(x, y) => {
             log.set(`pad context ${x} ${y}`);
             menu.set(true);
           }} />
      <Button label="Count" onPress={() => count.set(count() + 1)} />
      <Button label="Open dialog" onPress={() => dialog.set(true)} />
      <List items={["Apple"]} key={(s) => s} row={(s) => <Row title={s} onContextMenu={(x, y) => log.set(`row context ${s}`)} />} />
      <Menu label="Pad menu" open={menu} onClose={() => log.set(`${log()}, closed`)}
            actions={[<Action label="Rename" onPress={() => log.set("rename")} />, <Action label="Copy" onPress={() => log.set("copy")} />]} />
      <Dialog open={dialog} title="Confirm" message="Sure?" actions={[<Action label="OK" onPress={() => dialog.set(false)} />]} />
    </Screen>
  );
}

export default app({ screens: { home: { title: "Context", component: Home } } });
"#;

struct Harness {
    cx: HeadlessAppContext,
    window: WindowHandle<PlinthRoot>,
    scale: f64,
}

impl Harness {
    fn new() -> Self {
        let fs = MemFs::default().with("app/main.tsx", APP);
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
        let window = cx.open_window(size(px(800.), px(600.)), move |_, cx| cx.new(|cx| PlinthRoot::new(port, commits, "indigo", cx))).unwrap();
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

    /// The top-left corner of a node, in logical pixels.
    fn origin(&mut self, role: Role, name: &str) -> (f64, f64) {
        let b = self.bounds(role, name).unwrap_or_else(|| panic!("no {role:?} {name:?}"));
        (b.0, b.1)
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

    fn center(&mut self, role: Role, name: &str) -> Point<Pixels> {
        let (x0, y0, x1, y1) = self.bounds(role, name).unwrap_or_else(|| panic!("no {role:?} {name:?}"));
        point(px(((x0 + x1) / 2.0) as f32), px(((y0 + y1) / 2.0) as f32))
    }

    fn button(&mut self, button: MouseButton, p: Point<Pixels>) {
        self.cx
            .update_window(self.handle(), |_, window, cx| {
                window.dispatch_event(
                    PlatformInput::MouseDown(MouseDownEvent { position: p, modifiers: Default::default(), button, click_count: 1, first_mouse: false }),
                    cx,
                );
                window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { position: p, modifiers: Default::default(), button, click_count: 1 }), cx);
            })
            .unwrap();
        self.cx.run_until_parked();
    }

    fn click(&mut self, p: Point<Pixels>) {
        self.button(MouseButton::Left, p);
    }

    fn right_click(&mut self, p: Point<Pixels>) {
        self.button(MouseButton::Right, p);
    }

    fn key(&mut self, keys: &str) {
        for k in keys.split(' ') {
            let stroke = Keystroke::parse(k).unwrap();
            self.cx.update_window(self.handle(), |_, window, cx| window.dispatch_keystroke(stroke, cx)).unwrap();
            self.cx.run_until_parked();
        }
    }

    fn text(&mut self, prefix: &str) -> String {
        let tree = self.tree();
        tree.nodes.iter().filter_map(|(_, n)| n.label().or(n.value())).find(|l| l.starts_with(prefix)).unwrap_or("").to_owned()
    }

    fn log(&mut self) -> String {
        self.text("log: ")
    }

    fn menu_open(&mut self) -> bool {
        self.tree().nodes.iter().any(|(_, n)| n.role() == Role::Menu)
    }

    /// The focused node as `"<role> <name>"`.
    fn now(&mut self) -> String {
        let tree = self.tree();
        let n: Option<&Node> = tree.nodes.iter().find(|(i, _)| *i == tree.focus).map(|(_, n)| n);
        n.map(|n| format!("{:?} {}", n.role(), n.label().unwrap_or(""))).unwrap_or_else(|| "nothing".into())
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
}

/// A right-click on a box sends `onContextMenu` with the position in
/// spacing units; the menu that the handler opens shows at the pointer and
/// has the focus. Arrow keys move, Escape closes it (the app's signal
/// becomes false, `onClose` runs), an action closes it too, and a click
/// outside closes it.
#[test]
fn right_click_opens_the_menu_at_the_point() {
    let mut h = Harness::new();
    let pad = h.origin(Role::Group, "Pad");
    let at = point(px((pad.0 + 40.0) as f32), px((pad.1 + 20.0) as f32));
    h.right_click(at);
    assert_eq!(h.log(), "log: pad context 10 5", "the position is in spacing units");
    let (x0, y0, _, _) = h.bounds(Role::Menu, "Pad menu").expect("the menu is open");
    assert!((x0 - (pad.0 + 40.0)).abs() <= 1.0, "the menu is at the pointer: x {x0} for {}", pad.0 + 40.0);
    assert!((0.0..=6.0).contains(&(y0 - (pad.1 + 20.0))), "the menu is just below the pointer: y {y0} for {}", pad.1 + 20.0);
    assert_eq!(h.now(), "MenuItem Rename", "the open menu has the focus");
    h.key("down");
    assert_eq!(h.now(), "MenuItem Copy");
    h.key("escape");
    assert!(!h.menu_open(), "Escape closes the menu");
    assert_eq!(h.log(), "log: pad context 10 5, closed", "onClose runs");

    // An action runs, then the menu closes.
    h.right_click(at);
    assert!(h.menu_open(), "the signal became false, so the menu opens again");
    let copy = h.center(Role::MenuItem, "Copy");
    h.click(copy);
    assert!(!h.menu_open());
    assert_eq!(h.log(), "log: copy, closed");

    // A click outside closes it; the click also reaches what is under it.
    h.right_click(at);
    assert!(h.menu_open());
    let count = h.center(Role::Button, "Count");
    h.click(count);
    assert!(!h.menu_open(), "a click outside closes the menu");
    assert_eq!(h.text("count: "), "count: 1");

    // A right-click on a row of a List.
    let apple = h.center(Role::ListItem, "Apple");
    h.right_click(apple);
    assert_eq!(h.log(), "log: row context Apple");
}

/// Shift+F10 on the focused box sends `onContextMenu` with its center; the
/// menu opens there with the focus on its first item; Escape gives the
/// focus back to the box.
#[test]
fn shift_f10_opens_the_menu_at_the_center() {
    let mut h = Harness::new();
    h.tab_to("Group Pad");
    h.key("shift-f10");
    assert_eq!(h.log(), "log: pad context 25 12.5", "the center of the 50 x 25 box");
    assert_eq!(h.now(), "MenuItem Rename");
    let (pad_x0, pad_y0, pad_x1, pad_y1) = h.bounds(Role::Group, "Pad").unwrap();
    let (x0, y0, _, _) = h.bounds(Role::Menu, "Pad menu").expect("the menu is open");
    assert!((x0 - (pad_x0 + pad_x1) / 2.0).abs() <= 1.0, "the menu is at the center: {x0}");
    assert!((0.0..=6.0).contains(&(y0 - (pad_y0 + pad_y1) / 2.0)), "the menu is at the center: {y0}");
    h.key("escape");
    assert!(!h.menu_open());
    assert_eq!(h.now(), "Group Pad", "the focus goes back to the box");
    // The menu key does the same.
    h.key("menu");
    assert!(h.menu_open(), "the menu key opens it");
    h.key("enter");
    assert_eq!(h.log(), "log: rename, closed", "Enter presses the focused item");
    assert_eq!(h.now(), "Group Pad");
}

/// GAPS G4: while a Dialog is open, a click, a pointer-down and a
/// right-click on the elements under its backdrop do nothing to them.
#[test]
fn a_modal_dialog_blocks_the_pointer() {
    let mut h = Harness::new();
    let pad = h.origin(Role::Group, "Pad");
    let in_pad = point(px((pad.0 + 8.0) as f32), px((pad.1 + 8.0) as f32));
    // Before: the pad gets the pointer.
    h.click(in_pad);
    assert_eq!(h.log(), "log: pad down 2 2");

    let open = h.center(Role::Button, "Open dialog");
    h.click(open);
    assert!(h.bounds(Role::Dialog, "Confirm").is_some(), "the dialog is open");
    h.right_click(in_pad);
    assert_eq!(h.log(), "log: pad down 2 2", "no context menu under the dialog");
    assert!(!h.menu_open());
    let count = h.center(Role::Button, "Count");
    h.click(count);
    assert_eq!(h.text("count: "), "count: 0", "no click on the button under the dialog");
    // The click hit the panel or the backdrop (which closes the dialog).
    if h.bounds(Role::Dialog, "Confirm").is_some() {
        h.key("escape");
    }
    assert!(h.bounds(Role::Dialog, "Confirm").is_none());

    h.click(open);
    let other = point(px((pad.0 + 12.0) as f32), px((pad.1 + 12.0) as f32));
    h.click(other);
    assert_eq!(h.log(), "log: pad down 2 2", "no pointer-down under the dialog (it would say 3 3)");
    assert!(h.bounds(Role::Dialog, "Confirm").is_none());
    h.right_click(in_pad);
    assert_eq!(h.log(), "log: pad context 2 2", "after the dialog closes, the pad gets the pointer again");
}
