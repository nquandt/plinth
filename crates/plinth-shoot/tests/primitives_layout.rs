//! The Level 2 primitives (UI API 1.6, docs/UI-ADVANCED.md phase U1) on
//! the desktop: renders `examples/primitives` headless and checks the
//! AccessKit roles and the layout boxes, the same facts that
//! `web/test/run-a11y.mjs` (`checkPrimitives`) checks in the browser: the
//! six mapped cards of the row `Scroll` are in one row and 160 px wide
//! (40 spacing units), a `Pressable` is a button or a link with its label,
//! and the `Scroll` is a scroll view. Writes
//! `target/shots/primitives-regular.png` for a visual check.

use gpui::accesskit::Role;
use gpui::{AppContext as _, HeadlessAppContext, PlatformHeadlessRenderer, px, size};
use plinth_compiler::driver::DiskFs;
use plinth_runner_wasmtime::{Guest, Limits, Runner};
use plinth_ui::{GuestPort, PlinthRoot};
use std::path::PathBuf;

struct WasmGuest {
    guest: Guest,
    _runner: Runner,
}

impl GuestPort for WasmGuest {
    fn dispatch(&mut self, events: &[u8]) -> anyhow::Result<Vec<Vec<u8>>> {
        self.guest.on_event(events)
    }
}

#[test]
fn primitives_roles_and_layout() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/primitives");
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&DiskFs { root }, &[]).expect("compile");
    let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
    let artifact = artifact.unwrap_or_else(|| panic!("examples/primitives has errors:\n{}", diags.join("\n")));

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
    let window = cx.open_window(size(px(900.), px(1300.)), move |_, cx| cx.new(|cx| PlinthRoot::new(port, commits, "indigo", cx))).unwrap();
    cx.update(|cx| window.update(cx, |_, window, _| window.set_a11y_forced(true))).unwrap();
    cx.run_until_parked();

    let tree = cx.update(|cx| window.update(cx, |_, window, _| window.a11y_tree().cloned())).unwrap().unwrap();
    let found: Vec<_> =
        tree.nodes.iter().map(|(_, n)| (n.role(), n.label().or(n.value()).unwrap_or("").to_owned(), n.bounds())).collect();
    let find = |role: Role, name: &str| {
        found
            .iter()
            .find(|(r, l, _)| *r == role && l == name)
            .and_then(|(_, _, b)| *b)
            .unwrap_or_else(|| panic!("no {role:?} named {name:?}: {found:#?}"))
    };

    find(Role::Button, "Press me");
    find(Role::Link, "Reset the count");
    let scroll = find(Role::ScrollView, "Cards");
    assert!(found.iter().any(|(r, l, _)| *r == Role::Label && l == "Styled primitives"), "the Span is a label");

    // The cards: one row, each 40 units (160 px) wide, the first two side by
    // side inside the scroll view.
    // AccessKit bounds are in device pixels.
    let scale = f64::from(cx.update(|cx| window.update(cx, |_, window, _| window.scale_factor())).unwrap());
    let cards: Vec<_> = ["Inbox", "Calendar", "Notes", "Photos", "Music", "Weather"].iter().map(|n| find(Role::Button, n)).collect();
    for c in &cards {
        assert!((c.y0 - cards[0].y0).abs() < 0.5, "the cards are not in one row: {cards:?}");
        assert!(((c.x1 - c.x0) / scale - 160.0).abs() < 0.5, "a card is not 160 px wide (scale {scale}): {c:?}");
    }
    assert!(((cards[1].x0 - cards[0].x1) / scale - 12.0).abs() < 0.5, "the gap between cards is not 3 units: {cards:?}");
    assert!(cards[0].x0 >= scroll.x0 && cards[0].y0 >= scroll.y0, "the first card is not inside the scroll view");

    // UI API 1.7: in the regular width class the header box is a row (the
    // "U1" span is right of the title); its `compact` style makes it a column.
    let title = find(Role::Label, "Styled primitives");
    let badge = find(Role::Label, "U1");
    assert!(badge.x0 > title.x1 && (badge.y0 - title.y0).abs() < 40.0 * scale, "regular: the header is not a row: {title:?} {badge:?}");

    let shots = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/shots");
    std::fs::create_dir_all(&shots).unwrap();
    let image = cx.capture_screenshot(window.into()).expect("render the primitives");
    image.save(shots.join("primitives-regular.png")).expect("write the screenshot");

    // The same app in a compact window.
    let runner = Runner::new().unwrap();
    let mut guest = runner.load(&artifact.component, Limits::default()).unwrap();
    let commits = guest.init(&[]).unwrap();
    let port = Box::new(WasmGuest { guest, _runner: runner });
    let window = cx.open_window(size(px(400.), px(1300.)), move |_, cx| cx.new(|cx| PlinthRoot::new(port, commits, "indigo", cx))).unwrap();
    cx.update(|cx| window.update(cx, |_, window, _| window.set_a11y_forced(true))).unwrap();
    cx.run_until_parked();
    let tree = cx.update(|cx| window.update(cx, |_, window, _| window.a11y_tree().cloned())).unwrap().unwrap();
    let label_bounds = |name: &str| {
        tree.nodes
            .iter()
            .find(|(_, n)| n.role() == Role::Label && n.label() == Some(name))
            .and_then(|(_, n)| n.bounds())
            .unwrap_or_else(|| panic!("compact: no label {name:?}"))
    };
    let (title, badge) = (label_bounds("Styled primitives"), label_bounds("U1"));
    assert!(badge.y0 >= title.y1, "compact: the header is not a column: {title:?} {badge:?}");
    let image = cx.capture_screenshot(window.into()).expect("render the primitives");
    image.save(shots.join("primitives-compact.png")).expect("write the screenshot");
}
