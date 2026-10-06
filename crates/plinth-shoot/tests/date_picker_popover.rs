//! `<DatePicker>` (SPEC.md §6.3, UI API 1.4): rendered headless, its popover
//! must open as an anchored, deferred panel with an AccessKit `Grid` of
//! `GridCell` days, and picking a day must commit the new value through a
//! `change` event (no echo), mirroring `menu_popover.rs`.

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
import { app, signal, Screen, Text, DatePicker } from \"plinth:ui\";

function Home() {
  const due = signal(\"2026-10-06\");
  return (
    <Screen title=\"Home\">
      <Text>{`due: ${due()}`}</Text>
      <DatePicker label=\"Due\" value={due} />
    </Screen>
  );
}

export default app({ screens: { home: { title: \"Home\", component: Home } } });
";

fn build() -> plinth_compiler::Artifact {
    let fs = MemFs::default().with("app/main.tsx", APP);
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[]).expect("compile");
    let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
    artifact.unwrap_or_else(|| panic!("date picker popover test app has errors:\n{}", diags.join("\n")))
}

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
fn date_picker_popover_opens_grid_and_picks_a_day() {
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

    let scale = cx.update(|cx| window.update(cx, |_, window, _| window.scale_factor())).unwrap();
    let to_logical = |p: Point<Pixels>| point(p.x / scale, p.y / scale);

    // Closed: no Grid role yet, and the trigger (named with label + value)
    // is findable.
    let tree = cx.update(|cx| window.update(cx, |_, window, _| window.a11y_tree().cloned())).unwrap().unwrap();
    let found = nodes(&tree);
    assert!(!has(&found, gpui::accesskit::Role::Grid, ""), "grid should start closed: {found:#?}");
    let trigger = found
        .iter()
        .find(|(r, l, _)| *r == gpui::accesskit::Role::Button && l.contains("Due") && l.contains("Oct 6, 2026"))
        .unwrap_or_else(|| panic!("no Due trigger found: {found:#?}"));
    let trigger_pos = to_logical(center(trigger.2.expect("trigger has bounds")));

    let click = |cx: &mut HeadlessAppContext, pos: Point<Pixels>| {
        cx.update_window(handle, |_, window, cx| {
            window.dispatch_event(
                PlatformInput::MouseDown(MouseDownEvent {
                    position: pos,
                    modifiers: Default::default(),
                    button: MouseButton::Left,
                    click_count: 1,
                    first_mouse: false,
                }),
                cx,
            );
            window.dispatch_event(
                PlatformInput::MouseUp(MouseUpEvent { position: pos, modifiers: Default::default(), button: MouseButton::Left, click_count: 1 }),
                cx,
            );
        })
        .unwrap();
        cx.run_until_parked();
    };

    // Open: a Grid of GridCell days, including the 6th (selected) and
    // the 15th (a plain day in the same month).
    click(&mut cx, trigger_pos);
    let tree = cx.update(|cx| window.update(cx, |_, window, _| window.a11y_tree().cloned())).unwrap().unwrap();
    let found = nodes(&tree);
    assert!(has(&found, gpui::accesskit::Role::Grid, ""), "the popover should show a grid: {found:#?}");
    assert!(has(&found, gpui::accesskit::Role::GridCell, "Oct 6, 2026"), "the 6th should be a grid cell: {found:#?}");
    let fifteenth = found
        .iter()
        .find(|(r, l, _)| *r == gpui::accesskit::Role::GridCell && l.contains("Oct 15, 2026"))
        .unwrap_or_else(|| panic!("no Oct 15 grid cell found: {found:#?}"));
    let fifteenth_pos = to_logical(center(fifteenth.2.expect("grid cell has bounds")));

    // One PNG of the open popover (SPEC.md note: `plinth-shoot` itself
    // always closes popovers, since each shot reflects the initial state).
    let out = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/shots/dp-popover-open.png");
    let image = cx.capture_screenshot(window.into()).expect("render the open popover");
    image.save(&out).expect("write the popover screenshot");

    // Picking the 15th commits the new value (no echo) and closes the grid.
    click(&mut cx, fifteenth_pos);
    let tree = cx.update(|cx| window.update(cx, |_, window, _| window.a11y_tree().cloned())).unwrap().unwrap();
    let found = nodes(&tree);
    assert!(!has(&found, gpui::accesskit::Role::Grid, ""), "picking a day should close the grid: {found:#?}");
    assert!(has(&found, gpui::accesskit::Role::Label, "due: 2026-10-15"), "the signal should hold the new value: {found:#?}");
}
