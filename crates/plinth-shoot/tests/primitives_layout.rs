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
    // Canvas (UI API 1.10): an image with its label, in the aspect of its view space (200 x 100).
    let drawing = find(Role::Image, "A sun over five bars");
    let aspect = (drawing.x1 - drawing.x0) / (drawing.y1 - drawing.y0);
    assert!((aspect - 2.0).abs() < 0.02, "the canvas is not 2:1: {drawing:?}");
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

/// UI API 1.12: pointer events on the desktop, from gpui mouse events. A box
/// gives the position in spacing units from its top-left corner, a Canvas in
/// view units; after a pointer-down the element also gets the moves outside
/// it and the pointer-up. The same facts as `checkPong` in `run-a11y.mjs`.
#[test]
fn pointer_events_give_element_positions() {
    use gpui::{MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput, Pixels, Point, point};
    const APP: &str = r#"
import { app, signal, Screen, Box, Canvas, Text } from "plinth:ui";

function Home() {
  const log = signal("none");
  return (
    <Screen title="Pointer">
      <Box label="Pad" width={50} height={25} bg="surface"
           onPointerDown={(x, y) => log.set(`pad down ${x} ${y}`)}
           onPointerMove={(x, y) => log.set(`pad move ${x} ${y}`)}
           onPointerUp={(x, y) => log.set(`pad up ${x} ${y}`)} />
      <Canvas label="Sketch" viewWidth={100} viewHeight={50} width={50} shapes={[]}
              onPointerDown={(x, y) => log.set(`sketch down ${x} ${y}`)} />
      <Text>{log()}</Text>
    </Screen>
  );
}

export default app({ screens: { home: { title: "Pointer", component: Home } } });
"#;
    let fs = plinth_compiler::driver::MemFs::default().with("app/main.tsx", APP);
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[]).expect("compile");
    let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
    let artifact = artifact.unwrap_or_else(|| panic!("the pointer app has errors:\n{}", diags.join("\n")));

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
    let handle = window.into();
    cx.update(|cx| window.update(cx, |_, window, _| window.set_a11y_forced(true))).unwrap();
    cx.run_until_parked();

    // AccessKit bounds are device pixels; mouse positions are logical pixels.
    let scale = f64::from(cx.update(|cx| window.update(cx, |_, window, _| window.scale_factor())).unwrap());
    let tree = cx.update(|cx| window.update(cx, |_, window, _| window.a11y_tree().cloned())).unwrap().unwrap();
    let origin = |role: Role, name: &str| {
        let b = tree
            .nodes
            .iter()
            .find(|(_, n)| n.role() == role && n.label() == Some(name))
            .and_then(|(_, n)| n.bounds())
            .unwrap_or_else(|| panic!("no {role:?} {name:?}"));
        (b.x0 / scale, b.y0 / scale)
    };
    let pad = origin(Role::Group, "Pad");
    let sketch = origin(Role::Image, "Sketch");
    let at = |o: (f64, f64), dx: f64, dy: f64| -> Point<Pixels> { point(px((o.0 + dx) as f32), px((o.1 + dy) as f32)) };
    let send = |cx: &mut HeadlessAppContext, input: PlatformInput| {
        cx.update_window(handle, |_, window, cx| {
            window.dispatch_event(input, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    let down = |p| PlatformInput::MouseDown(MouseDownEvent { position: p, modifiers: Default::default(), button: MouseButton::Left, click_count: 1, first_mouse: false });
    let mv = |p, held: bool| PlatformInput::MouseMove(MouseMoveEvent { position: p, pressed_button: held.then_some(MouseButton::Left), modifiers: Default::default() });
    let up = |p| PlatformInput::MouseUp(MouseUpEvent { position: p, modifiers: Default::default(), button: MouseButton::Left, click_count: 1 });
    let log = |cx: &mut HeadlessAppContext| {
        let tree = cx.update(|cx| window.update(cx, |_, window, _| window.a11y_tree().cloned())).unwrap().unwrap();
        tree.nodes
            .iter()
            .filter_map(|(_, n)| n.label().or(n.value()).map(str::to_owned))
            .find(|l| l.starts_with("pad ") || l.starts_with("sketch ") || l == "none")
            .unwrap_or_default()
    };

    // A box: spacing units (4 px) from its top-left corner.
    send(&mut cx, mv(at(pad, 10.0, 10.0), false));
    assert_eq!(log(&mut cx), "pad move 2.5 2.5", "a move over the box");
    send(&mut cx, down(at(pad, 40.0, 20.0)));
    assert_eq!(log(&mut cx), "pad down 10 5");
    // While the button is down, the box gets the moves outside it and the up.
    send(&mut cx, mv(at(pad, 400.0, 300.0), true));
    assert_eq!(log(&mut cx), "pad move 100 75", "a drag outside the box");
    send(&mut cx, up(at(pad, 400.0, 300.0)));
    assert_eq!(log(&mut cx), "pad up 100 75");
    send(&mut cx, mv(at(pad, 404.0, 300.0), false));
    assert_eq!(log(&mut cx), "pad up 100 75", "no moves outside the box after the up");

    // A Canvas: view units (the 100 x 50 view is 200 px wide: 2 px a unit).
    send(&mut cx, down(at(sketch, 30.0, 12.0)));
    assert_eq!(log(&mut cx), "sketch down 15 6");
    send(&mut cx, up(at(sketch, 30.0, 12.0)));
}

/// 7GUIs task 6 on the desktop: real clicks on the canvas draw circles (the
/// canvas label counts them), a click in a circle draws none. Writes
/// `target/shots/circle-drawer.png` for a visual check.
#[test]
fn circle_drawer_draws_where_clicked() {
    use gpui::{MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput, point};
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/7guis/circle-drawer");
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&DiskFs { root }, &[]).expect("compile");
    let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
    let artifact = artifact.unwrap_or_else(|| panic!("the circle drawer has errors:\n{}", diags.join("\n")));
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
    let window = cx.open_window(size(px(900.), px(800.)), move |_, cx| cx.new(|cx| PlinthRoot::new(port, commits, "indigo", cx))).unwrap();
    let handle = window.into();
    cx.update(|cx| window.update(cx, |_, window, _| window.set_a11y_forced(true))).unwrap();
    cx.run_until_parked();

    let scale = f64::from(cx.update(|cx| window.update(cx, |_, window, _| window.scale_factor())).unwrap());
    let drawing = |cx: &mut HeadlessAppContext| {
        let tree = cx.update(|cx| window.update(cx, |_, window, _| window.a11y_tree().cloned())).unwrap().unwrap();
        tree.nodes
            .iter()
            .find(|(_, n)| n.role() == Role::Image)
            .map(|(_, n)| (n.label().unwrap_or("").to_owned(), n.bounds().unwrap()))
            .expect("the canvas")
    };
    let (label, b) = drawing(&mut cx);
    assert!(label.starts_with("Drawing with 0 circles"), "{label}");
    // The view is 400 units wide.
    let k = (b.x1 - b.x0) / scale / 400.0;
    let click = |cx: &mut HeadlessAppContext, vx: f64, vy: f64| {
        let p = point(px((b.x0 / scale + vx * k) as f32), px((b.y0 / scale + vy * k) as f32));
        cx.update_window(handle, |_, window, cx| {
            window.dispatch_event(
                PlatformInput::MouseDown(MouseDownEvent { position: p, modifiers: Default::default(), button: MouseButton::Left, click_count: 1, first_mouse: false }),
                cx,
            );
            window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { position: p, modifiers: Default::default(), button: MouseButton::Left, click_count: 1 }), cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    click(&mut cx, 80.0, 80.0);
    click(&mut cx, 200.0, 150.0);
    click(&mut cx, 300.0, 220.0);
    click(&mut cx, 205.0, 150.0);
    let (label, _) = drawing(&mut cx);
    assert!(label.starts_with("Drawing with 3 circles"), "three clicks on empty canvas, one in a circle: {label}");

    let shots = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/shots");
    std::fs::create_dir_all(&shots).unwrap();
    let image = cx.capture_screenshot(window.into()).expect("render the circle drawer");
    image.save(shots.join("circle-drawer.png")).expect("write the screenshot");
}

/// 7GUIs task 7 on the desktop (docs/VALIDATION.md §2: "a scroll through
/// 26 x 100 cells stays under 16 ms for each frame"): scrolls the rows with
/// wheel events and times `Window::draw` (render, layout, prepaint, paint)
/// for each frame. Prints the numbers; asserts only that the rows scroll,
/// because a debug build is much slower than a release build. Run with
/// `--release -- --nocapture` for the VALIDATION numbers.
#[test]
fn cells_scroll_frame_times() {
    use gpui::{PlatformInput, ScrollDelta, ScrollWheelEvent, TouchPhase, point};
    use std::time::Instant;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/7guis/cells");
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&DiskFs { root }, &[]).expect("compile");
    let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
    let artifact = artifact.unwrap_or_else(|| panic!("cells has errors:\n{}", diags.join("\n")));
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
    let window = cx.open_window(size(px(1000.), px(800.)), move |_, cx| cx.new(|cx| PlinthRoot::new(port, commits, "indigo", cx))).unwrap();
    let handle = window.into();
    cx.update(|cx| window.update(cx, |_, window, _| window.set_a11y_forced(true))).unwrap();
    cx.run_until_parked();

    let scale = f64::from(cx.update(|cx| window.update(cx, |_, window, _| window.scale_factor())).unwrap());
    let bounds = |cx: &mut HeadlessAppContext, role: Role, name: &str| {
        let tree = cx.update(|cx| window.update(cx, |_, window, _| window.a11y_tree().cloned())).unwrap().unwrap();
        tree.nodes.iter().find(|(_, n)| n.role() == role && n.label() == Some(name)).and_then(|(_, n)| n.bounds()).unwrap_or_else(|| panic!("no {role:?} {name:?}"))
    };
    // The pointer over cell B5: the "Rows" box is as wide as all 26
    // columns (most of it is outside the window), so not its center.
    let b5 = bounds(&mut cx, Role::Button, "B5");
    let a5_before = bounds(&mut cx, Role::Button, "A5").y0;
    let over = point(px(((b5.x0 + b5.x1) / 2.0 / scale) as f32), px(((b5.y0 + b5.y1) / 2.0 / scale) as f32));
    let draw = |cx: &mut HeadlessAppContext| -> f64 {
        cx.update_window(handle, |_, window, cx| {
            window.refresh();
            let t = Instant::now();
            let arena = window.draw(cx);
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            arena.clear(cx);
            ms
        })
        .unwrap()
    };
    // The pointer goes over the rows first: gpui scrolls the hovered box.
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_event(PlatformInput::MouseMove(gpui::MouseMoveEvent { position: over, pressed_button: None, modifiers: Default::default() }), cx);
    })
    .unwrap();
    // Warm up (text shaping caches), then scroll 30 frames of two lines.
    for _ in 0..3 {
        draw(&mut cx);
    }
    // 30 frames with AccessKit on (as with a screen reader), then 30 with it
    // off (the normal case); the checks below need it on again.
    let mut times = Vec::new();
    let mut plain = Vec::new();
    for frame in 0..60 {
        if frame == 30 {
            cx.update(|cx| window.update(cx, |_, window, _| window.set_a11y_forced(false))).unwrap();
        }
        cx.update_window(handle, |_, window, cx| {
            window.dispatch_event(
                PlatformInput::ScrollWheel(ScrollWheelEvent {
                    position: over,
                    delta: ScrollDelta::Lines(point(0., -2.)),
                    modifiers: Default::default(),
                    touch_phase: TouchPhase::Moved,
                }),
                cx,
            );
        })
        .unwrap();
        cx.run_until_parked();
        let ms = draw(&mut cx);
        if frame < 30 { times.push(ms) } else { plain.push(ms) }
    }
    cx.update(|cx| window.update(cx, |_, window, _| window.set_a11y_forced(true))).unwrap();
    draw(&mut cx);
    cx.run_until_parked();
    let shots = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/shots");
    std::fs::create_dir_all(&shots).unwrap();
    cx.capture_screenshot(handle).expect("render cells").save(shots.join("cells-scrolled.png")).expect("write the screenshot");
    // The rows scrolled: row 5 is no longer built (the renderer builds only
    // the rows near the viewport, GAPS 7G-12), and a later row is in view
    // where row 5 was, give or take a few rows.
    let tree = cx.update(|cx| window.update(cx, |_, window, _| window.a11y_tree().cloned())).unwrap().unwrap();
    // The wheel also scrolls the outer, horizontal scroll, so any column.
    let cells_built: Vec<(i32, f64)> = tree
        .nodes
        .iter()
        .filter_map(|(_, n)| {
            let l = n.label()?;
            let row = l.get(1..)?.parse::<i32>().ok().filter(|_| n.role() == Role::Button && l.starts_with(|c: char| c.is_ascii_uppercase()))?;
            Some((row, n.bounds().map_or(0.0, |b| b.y0)))
        })
        .collect();
    let mut rows_built: Vec<(i32, f64)> = cells_built.clone();
    rows_built.dedup_by_key(|r| r.0);
    eprintln!("cells built: {} in {} rows", cells_built.len(), rows_built.len());
    assert!(cells_built.len() < 26 * 30, "only the cells near the viewport are built: {}", cells_built.len());
    assert!(!rows_built.iter().any(|(r, _)| *r == 5), "row 5 scrolled out");
    let (row, y) = rows_built.iter().min_by(|a, b| (a.1 - a5_before).abs().total_cmp(&(b.1 - a5_before).abs())).copied().unwrap();
    assert!(row > 20, "a later row is where row 5 was: row {row} at {y}");
    times.sort_by(f64::total_cmp);
    plain.sort_by(f64::total_cmp);
    let build = if cfg!(debug_assertions) { "debug" } else { "release" };
    eprintln!(
        "cells scroll ({build} build, AccessKit off): draw median {:.1} ms, p95 {:.1} ms, max {:.1} ms over {} frames",
        plain[plain.len() / 2],
        plain[plain.len() * 95 / 100],
        plain[plain.len() - 1],
        plain.len()
    );
    eprintln!(
        "cells scroll ({build} build, AccessKit on): draw median {:.1} ms, p95 {:.1} ms, max {:.1} ms over {} frames",
        times[times.len() / 2],
        times[times.len() * 95 / 100],
        times[times.len() - 1],
        times.len()
    );
}

