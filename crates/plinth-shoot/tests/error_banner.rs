//! The host error banner (SPEC.md §5.6): an uncaught app exception goes to
//! the host through `error.report`; the desktop host shows the last one in
//! a banner with a "Dismiss" button, and the app keeps running.

use gpui::{AppContext as _, HeadlessAppContext, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, PlatformHeadlessRenderer, PlatformInput, Point, point, px, size};
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
      <Button label=\"Boom\" onPress={() => { n.set(n() + 1); throw new Error(`boom ${n()}`); }} />
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

#[test]
fn an_uncaught_error_shows_a_dismissible_banner() {
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

    let found = tree(&mut cx);
    assert!(find(&found, gpui::accesskit::Role::Alert, "error").is_none(), "no banner at start: {found:#?}");
    let boom = to_logical(center(find(&found, gpui::accesskit::Role::Button, "Boom").expect("Boom button")));

    click(&mut cx, boom);
    // The banner moves the screen down: find the button again.
    let found = tree(&mut cx);
    assert!(find(&found, gpui::accesskit::Role::Alert, "Uncaught Error: boom 1").is_some(), "banner shows the error: {found:#?}");
    let boom = to_logical(center(find(&found, gpui::accesskit::Role::Button, "Boom").expect("Boom button")));
    click(&mut cx, boom);
    let found = tree(&mut cx);
    assert!(find(&found, gpui::accesskit::Role::Alert, "Uncaught Error: boom 2").is_some(), "banner shows the last error: {found:#?}");
    assert!(find(&found, gpui::accesskit::Role::Alert, "(1 more)").is_some(), "banner counts the others: {found:#?}");
    assert!(found.iter().any(|(_, l, _)| l.contains("count: 2")), "the app keeps running: {found:#?}");

    let dismiss = to_logical(center(find(&found, gpui::accesskit::Role::Button, "Dismiss error").expect("Dismiss button")));
    click(&mut cx, dismiss);
    let found = tree(&mut cx);
    assert!(find(&found, gpui::accesskit::Role::Alert, "error").is_none(), "dismissed: {found:#?}");
    let errors = cx.update(|cx| window.update(cx, |root, _, _| root.app_errors().len())).unwrap();
    assert_eq!(errors, 0);
}
