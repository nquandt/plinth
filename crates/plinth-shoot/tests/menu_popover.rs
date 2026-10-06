//! The `<Menu>` popover (SPEC.md §6.3): rendered headless, it must open as
//! an anchored, deferred panel with AccessKit `Menu`/`MenuItem` roles, close
//! on an outside click, close on Escape, and close (and fire its handler)
//! when an action is pressed.

use gpui::{AppContext as _, HeadlessAppContext, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, PlatformInput, PlatformHeadlessRenderer, Point, point, px, size};
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

const APP: &str = "\
import { app, signal, Screen, Text, Menu, Action } from \"plinth:ui\";

function Home() {
  const picked = signal(\"none\");
  return (
    <Screen title=\"Home\">
      <Text>{`picked: ${picked()}`}</Text>
      <Menu
        label=\"Actions\"
        actions={[
          <Action label=\"Share\" onPress={() => picked.set(\"share\")} />,
          <Action label=\"Rename\" onPress={() => picked.set(\"rename\")} />,
          <Action label=\"Archive\" onPress={() => picked.set(\"archive\")} />,
        ]}
      />
    </Screen>
  );
}

export default app({ screens: { home: { title: \"Home\", component: Home } } });
";

fn build() -> plinth_compiler::Artifact {
    let fs = MemFs::default().with("app/main.tsx", APP);
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[]).expect("compile");
    let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
    artifact.unwrap_or_else(|| panic!("menu popover test app has errors:\n{}", diags.join("\n")))
}

/// Collects `(role, label, bounds)` for every a11y node.
fn nodes(tree: &gpui::accesskit::TreeUpdate) -> Vec<(gpui::accesskit::Role, String, Option<gpui::accesskit::Rect>)> {
    tree.nodes.iter().map(|(_, n)| (n.role(), n.label().or(n.value()).unwrap_or("").to_owned(), n.bounds())).collect()
}

fn has(found: &[(gpui::accesskit::Role, String, Option<gpui::accesskit::Rect>)], role: gpui::accesskit::Role, name: &str) -> bool {
    found.iter().any(|(r, l, _)| *r == role && l.contains(name))
}

fn center(r: gpui::accesskit::Rect) -> Point<Pixels> {
    point(px(((r.x0 + r.x1) / 2.0) as f32), px(((r.y0 + r.y1) / 2.0) as f32))
}

#[test]
fn menu_popover_opens_closes_and_fires_actions() {
    let artifact = build();

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

    // AccessKit bounds are in scaled (device) pixels; `dispatch_event`
    // positions are in logical pixels (SPEC.md §9.3 roles, §6.3 popovers).
    let scale = cx.update(|cx| window.update(cx, |_, window, _| window.scale_factor())).unwrap();
    let to_logical = |p: Point<Pixels>| point(p.x / scale, p.y / scale);

    // Closed: no Menu/MenuItem roles, and the trigger is findable.
    let tree = cx.update(|cx| window.update(cx, |_, window, _| window.a11y_tree().cloned())).unwrap().unwrap();
    let found = nodes(&tree);
    assert!(!has(&found, gpui::accesskit::Role::Menu, ""), "menu should start closed: {found:#?}");
    let trigger = found
        .iter()
        .find(|(r, l, _)| *r == gpui::accesskit::Role::Button && l.contains("Actions"))
        .unwrap_or_else(|| panic!("no Actions trigger found: {found:#?}"));
    let trigger_pos = to_logical(center(trigger.2.expect("trigger has bounds")));

    // Click the trigger: the panel opens with Menu/MenuItem roles.
    let click = |cx: &mut HeadlessAppContext, pos: Point<Pixels>| {
        cx.update_window(handle, |_, window, cx| {
            let r1 = window.dispatch_event(
                PlatformInput::MouseDown(MouseDownEvent {
                    position: pos,
                    modifiers: Default::default(),
                    button: MouseButton::Left,
                    click_count: 1,
                    first_mouse: false,
                }),
                cx,
            );
            let r2 = window.dispatch_event(
                PlatformInput::MouseUp(MouseUpEvent { position: pos, modifiers: Default::default(), button: MouseButton::Left, click_count: 1 }),
                cx,
            );
            let _ = (r1, r2);
        })
        .unwrap();
        cx.run_until_parked();
    };

    click(&mut cx, trigger_pos);
    let tree = cx.update(|cx| window.update(cx, |_, window, _| window.a11y_tree().cloned())).unwrap().unwrap();
    let found = nodes(&tree);
    assert!(has(&found, gpui::accesskit::Role::Menu, ""), "menu should be open: {found:#?}");
    assert!(has(&found, gpui::accesskit::Role::MenuItem, "Share"), "Share should be a menu item: {found:#?}");
    assert!(has(&found, gpui::accesskit::Role::MenuItem, "Rename"), "Rename should be a menu item: {found:#?}");
    assert!(has(&found, gpui::accesskit::Role::MenuItem, "Archive"), "Archive should be a menu item: {found:#?}");

    // An outside click closes it.
    click(&mut cx, point(px(10.), px(10.)));
    let tree = cx.update(|cx| window.update(cx, |_, window, _| window.a11y_tree().cloned())).unwrap().unwrap();
    let found = nodes(&tree);
    assert!(!has(&found, gpui::accesskit::Role::Menu, ""), "outside click should close the menu: {found:#?}");

    // Escape closes it too.
    click(&mut cx, trigger_pos);
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_event(
            PlatformInput::KeyDown(gpui::KeyDownEvent {
                keystroke: gpui::Keystroke { modifiers: Default::default(), key: "escape".into(), key_char: None },
                is_held: false,
                prefer_character_input: false,
            }),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    let tree = cx.update(|cx| window.update(cx, |_, window, _| window.a11y_tree().cloned())).unwrap().unwrap();
    let found = nodes(&tree);
    assert!(!has(&found, gpui::accesskit::Role::Menu, ""), "escape should close the menu: {found:#?}");

    // Pressing an action closes the menu and fires its handler.
    click(&mut cx, trigger_pos);
    let tree = cx.update(|cx| window.update(cx, |_, window, _| window.a11y_tree().cloned())).unwrap().unwrap();
    let found = nodes(&tree);
    let rename = found
        .iter()
        .find(|(r, l, _)| *r == gpui::accesskit::Role::MenuItem && l.contains("Rename"))
        .unwrap_or_else(|| panic!("no Rename menu item found: {found:#?}"));
    let rename_pos = to_logical(center(rename.2.expect("menu item has bounds")));
    click(&mut cx, rename_pos);

    let tree = cx.update(|cx| window.update(cx, |_, window, _| window.a11y_tree().cloned())).unwrap().unwrap();
    let found = nodes(&tree);
    assert!(!has(&found, gpui::accesskit::Role::Menu, ""), "pressing an action should close the menu: {found:#?}");
    assert!(has(&found, gpui::accesskit::Role::Label, "picked: rename"), "the action's handler should have fired: {found:#?}");
}
