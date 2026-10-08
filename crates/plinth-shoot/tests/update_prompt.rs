//! The host-owned update prompt (`docs/UPDATES.md` §5, `apply =
//! "prompt"`): the desktop host shows "An update is ready. Restart now?"
//! with "Restart" and "Later" at the top of the app window. "Later" only
//! removes it; "Restart" removes it and calls the host's restart action.
//! The app stays usable while the prompt shows.

use gpui::{AppContext as _, HeadlessAppContext, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, PlatformHeadlessRenderer, PlatformInput, Point, point, px, size};
use plinth_compiler::driver::MemFs;
use plinth_runner_wasmtime::{Guest, Limits, Runner};
use plinth_ui::{GuestPort, PlinthRoot};
use std::cell::Cell;
use std::rc::Rc;

struct WasmGuest {
    guest: Guest,
    _runner: Runner,
}

impl GuestPort for WasmGuest {
    fn dispatch(&mut self, events: &[u8]) -> anyhow::Result<Vec<Vec<u8>>> {
        self.guest.on_event(events)
    }

    fn take_errors(&mut self) -> Vec<String> {
        self.guest.take_errors()
    }
}

const APP: &str = "\
import { app, signal, Screen, Text, Button } from \"plinth:ui\";

function Home() {
  const n = signal(0);
  return (
    <Screen title=\"Home\">
      <Text>{`count: ${n()}`}</Text>
      <Button label=\"Add\" onPress={() => n.set(n() + 1)} />
    </Screen>
  );
}

export default app({ screens: { home: { title: \"Home\", component: Home } } });
";

type Found = Vec<(gpui::accesskit::Role, String, Option<gpui::accesskit::Rect>)>;

fn nodes(tree: &gpui::accesskit::TreeUpdate) -> Found {
    tree.nodes.iter().map(|(_, n)| (n.role(), n.label().or(n.value()).unwrap_or("").to_owned(), n.bounds())).collect()
}

fn find(found: &Found, role: gpui::accesskit::Role, name: &str) -> Option<gpui::accesskit::Rect> {
    found.iter().find(|(r, l, _)| *r == role && l.contains(name)).and_then(|(_, _, b)| *b)
}

fn center(r: gpui::accesskit::Rect) -> Point<Pixels> {
    point(px(((r.x0 + r.x1) / 2.0) as f32), px(((r.y0 + r.y1) / 2.0) as f32))
}

const PROMPT: &str = "An update is ready. Restart now?";

#[test]
fn the_update_prompt_shows_restart_and_later() {
    let fs = MemFs::default().with("app/main.tsx", APP);
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[]).expect("compile");
    let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
    let artifact = artifact.unwrap_or_else(|| panic!("test app has errors:\n{}", diags.join("\n")));

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
    let window =
        cx.open_window(size(px(900.), px(760.)), move |_, cx| cx.new(|cx| PlinthRoot::new(port, commits, "teal", cx))).unwrap();
    let handle = window.into();
    cx.update(|cx| window.update(cx, |_, window, _| window.set_a11y_forced(true))).unwrap();
    cx.run_until_parked();
    let scale = cx.update(|cx| window.update(cx, |_, window, _| window.scale_factor())).unwrap();
    let to_logical = |p: Point<Pixels>| point(p.x / scale, p.y / scale);
    let click = |cx: &mut HeadlessAppContext, pos: Point<Pixels>| {
        cx.update_window(handle, |_, window, cx| {
            let down = MouseDownEvent { position: pos, modifiers: Default::default(), button: MouseButton::Left, click_count: 1, first_mouse: false };
            window.dispatch_event(PlatformInput::MouseDown(down), cx);
            let up = MouseUpEvent { position: pos, modifiers: Default::default(), button: MouseButton::Left, click_count: 1 };
            window.dispatch_event(PlatformInput::MouseUp(up), cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    let tree = |cx: &mut HeadlessAppContext| nodes(&cx.update(|cx| window.update(cx, |_, window, _| window.a11y_tree().cloned())).unwrap().unwrap());
    let restarts = Rc::new(Cell::new(0));
    let show = |cx: &mut HeadlessAppContext| {
        let restarts = restarts.clone();
        cx.update(|cx| window.update(cx, |root, _, cx| root.show_update_prompt(move |_| restarts.set(restarts.get() + 1), cx))).unwrap();
        cx.run_until_parked();
    };

    let found = tree(&mut cx);
    assert!(find(&found, gpui::accesskit::Role::Status, PROMPT).is_none(), "no prompt at start: {found:#?}");

    // "Later" removes the prompt and does not restart.
    show(&mut cx);
    let found = tree(&mut cx);
    assert!(find(&found, gpui::accesskit::Role::Status, PROMPT).is_some(), "the prompt shows: {found:#?}");
    // The app stays usable while the prompt shows (the prompt is not modal).
    let add = to_logical(center(find(&found, gpui::accesskit::Role::Button, "Add").expect("Add button")));
    click(&mut cx, add);
    let found = tree(&mut cx);
    assert!(found.iter().any(|(_, l, _)| l.contains("count: 1")), "the app is usable: {found:#?}");
    let later = to_logical(center(find(&found, gpui::accesskit::Role::Button, "Later").expect("Later button")));
    click(&mut cx, later);
    let found = tree(&mut cx);
    assert!(find(&found, gpui::accesskit::Role::Status, PROMPT).is_none(), "Later removes the prompt: {found:#?}");
    assert_eq!(restarts.get(), 0, "Later does not restart");

    // "Restart" removes the prompt and calls the host's restart action once.
    show(&mut cx);
    let found = tree(&mut cx);
    let restart = to_logical(center(find(&found, gpui::accesskit::Role::Button, "Restart").expect("Restart button")));
    click(&mut cx, restart);
    let found = tree(&mut cx);
    assert!(find(&found, gpui::accesskit::Role::Status, PROMPT).is_none(), "Restart removes the prompt: {found:#?}");
    assert_eq!(restarts.get(), 1, "Restart calls the restart action");
    let shown = cx.update(|cx| window.update(cx, |root, _, _| root.update_prompt_shown())).unwrap();
    assert!(!shown);
}
