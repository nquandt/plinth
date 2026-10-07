//! The frame cost of the desktop renderer for every example (HANDOFF §9
//! step 6): each app opens in a headless window at 1000 x 800, and the
//! test draws 20 frames after a warm-up and prints the median and the
//! maximum of `Window::draw`. A frame of a normal screen must stay far
//! below the 8 ms of a 120 Hz display; the test fails above a generous
//! limit, so that a large regression shows.
//!
//! `PLINTH_FRAME_APP=<part of a name>` measures only matching apps.

use gpui::{AppContext as _, HeadlessAppContext, PlatformHeadlessRenderer, px, size};
use plinth_compiler::driver::DiskFs;
use plinth_runner_wasmtime::{Guest, Limits, Runner};
use plinth_ui::{GuestPort, PlinthRoot};
use std::path::PathBuf;
use std::time::Instant;

struct WasmGuest {
    guest: Guest,
    _runner: Runner,
}

impl GuestPort for WasmGuest {
    fn dispatch(&mut self, events: &[u8]) -> anyhow::Result<Vec<Vec<u8>>> {
        self.guest.on_event(events)
    }
}

fn examples() -> Vec<(String, PathBuf)> {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut out = Vec::new();
    for e in std::fs::read_dir(&base).unwrap().flatten() {
        let p = e.path();
        if p.join("app/main.tsx").is_file() {
            out.push(p);
        } else if p.is_dir() {
            out.extend(std::fs::read_dir(&p).unwrap().flatten().map(|e| e.path()).filter(|p| p.join("app/main.tsx").is_file()));
        }
    }
    out.sort();
    out.into_iter().map(|p| (p.strip_prefix(&base).unwrap().to_string_lossy().replace('\\', "/"), p)).collect()
}

fn capabilities(root: &std::path::Path) -> Vec<String> {
    let text = std::fs::read_to_string(root.join("plinth.toml")).unwrap_or_default();
    text.parse::<toml::Value>()
        .ok()
        .and_then(|v| v.get("capabilities")?.as_array().cloned())
        .map(|caps| caps.iter().filter_map(|c| c.get("name")?.as_str().map(str::to_owned)).collect())
        .unwrap_or_default()
}

#[test]
fn every_example_draws_a_frame_fast() {
    let only = std::env::var("PLINTH_FRAME_APP").ok();
    let platform = gpui_platform::current_platform(true);
    let mut cx = HeadlessAppContext::with_platform(platform.text_system(), std::sync::Arc::new(()), || {
        gpui_wgpu::WgpuHeadlessRenderer::new()
            .map(|r| Box::new(r) as Box<dyn PlatformHeadlessRenderer>)
            .map_err(|e| log::error!("no headless renderer: {e:#}"))
            .ok()
    });
    cx.update(plinth_ui::init);
    let mut rows = Vec::new();
    for (name, root) in examples() {
        if only.as_deref().is_some_and(|o| !name.contains(o)) {
            continue;
        }
        let caps = capabilities(&root);
        let (_, artifact) = plinth_compiler::compile_with_capabilities(&DiskFs { root }, &caps).expect("compile");
        let Some(artifact) = artifact else { panic!("{name} does not build") };
        let runner = Runner::new().unwrap();
        let mut guest = runner.load(&artifact.component, Limits::default()).unwrap();
        let commits = guest.init(&[]).unwrap();
        let nodes = {
            let mut t = plinth_ui::tree::Tree::new();
            for c in &commits {
                t.apply(c).unwrap();
            }
            t.len()
        };
        let port = Box::new(WasmGuest { guest, _runner: runner });
        let window = cx.open_window(size(px(1000.), px(800.)), move |_, cx| cx.new(|cx| PlinthRoot::new(port, commits, "teal", cx))).unwrap();
        let handle = window.into();
        cx.run_until_parked();
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
        for _ in 0..3 {
            draw(&mut cx);
        }
        let mut times: Vec<f64> = (0..20).map(|_| draw(&mut cx)).collect();
        times.sort_by(f64::total_cmp);
        rows.push((name, nodes, times[10], times[19]));
        cx.update(|cx| window.update(cx, |_, window, _| window.remove_window())).unwrap();
        cx.run_until_parked();
    }
    rows.sort_by(|a, b| b.2.total_cmp(&a.2));
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    eprintln!("frame cost ({profile} build, 1000 x 800, AccessKit off), slowest first:");
    for (name, nodes, median, max) in &rows {
        eprintln!("  {name:<24} {nodes:>6} nodes  median {median:>7.2} ms  max {max:>7.2} ms");
    }
    // Generous: a debug build of a large screen is slow. The limit catches
    // a regression of an order of magnitude, not a small one.
    let limit = if cfg!(debug_assertions) { 400.0 } else { 40.0 };
    for (name, _, median, _) in &rows {
        assert!(*median < limit, "{name}: median frame {median:.1} ms (limit {limit} ms)");
    }
}

/// The cost of one control: a screen with five of the same control, for the
/// controls of the slow examples. It prints a table and checks nothing; it
/// finds the control to fix when `every_example_draws_a_frame_fast` grows.
#[test]
fn frame_cost_per_control() {
    let controls: &[(&str, &str)] = &[
        ("Text", r#"<Text>Some text in a paragraph</Text>"#),
        ("Button", r#"<Button label="Press" onPress={() => {}} />"#),
        ("TextField", r#"<TextField label="Name" value={s} />"#),
        ("TextArea", r#"<TextArea label="Notes" value={s} />"#),
        ("NumberField", r#"<NumberField label="Amount" value={n} />"#),
        ("Toggle", r#"<Toggle label="On" value={b} />"#),
        ("Slider", r#"<Slider label="Level" min={0} max={10} value={n} />"#),
        ("Picker", r#"<Picker label="Kind" options={["One", "Two", "Three"]} value={s} />"#),
        ("DatePicker", r#"<DatePicker label="Day" value={d} />"#),
        ("Progress", r#"<Progress label="Done" value={0.5} />"#),
    ];
    let platform = gpui_platform::current_platform(true);
    let mut cx = HeadlessAppContext::with_platform(platform.text_system(), std::sync::Arc::new(()), || {
        gpui_wgpu::WgpuHeadlessRenderer::new().map(|r| Box::new(r) as Box<dyn PlatformHeadlessRenderer>).ok()
    });
    cx.update(plinth_ui::init);
    eprintln!("frame cost per control ({} build, five of each):", if cfg!(debug_assertions) { "debug" } else { "release" });
    for (name, jsx) in controls {
        let main = format!(
            "import {{ app, signal, Screen, Section, Text, Button, TextField, TextArea, NumberField, Toggle, Slider, Picker, DatePicker, Progress }} from \"plinth:ui\";\n\
             const s = signal(\"One\"); const n = signal(3); const b = signal(true); const d = signal(\"2026-10-07\");\n\
             function Home() {{ return <Screen title=\"C\"><Section title=\"S\">{jsx}{jsx}{jsx}{jsx}{jsx}</Section></Screen>; }}\n\
             export default app({{ screens: {{ home: {{ title: \"C\", component: Home }} }} }});\n"
        );
        let fs = plinth_compiler::driver::MemFs::default().with("app/main.tsx", &main);
        let (front, artifact) = plinth_compiler::compile(&fs).unwrap();
        let Some(artifact) = artifact else {
            eprintln!("  {name}: does not build: {:?}", front.diags.iter().map(|d| front.sources.render(d)).collect::<Vec<_>>());
            continue;
        };
        let runner = Runner::new().unwrap();
        let mut guest = runner.load(&artifact.component, Limits::default()).unwrap();
        let commits = guest.init(&[]).unwrap();
        let port = Box::new(WasmGuest { guest, _runner: runner });
        let window = cx.open_window(size(px(1000.), px(800.)), move |_, cx| cx.new(|cx| PlinthRoot::new(port, commits, "teal", cx))).unwrap();
        let handle = window.into();
        cx.run_until_parked();
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
        for _ in 0..3 {
            draw(&mut cx);
        }
        let mut times: Vec<f64> = (0..15).map(|_| draw(&mut cx)).collect();
        times.sort_by(f64::total_cmp);
        eprintln!("  {name:<12} median {:>7.2} ms", times[7]);
        cx.update(|cx| window.update(cx, |_, window, _| window.remove_window())).unwrap();
        cx.run_until_parked();
    }
}
