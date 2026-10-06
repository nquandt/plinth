//! `<Chart>` (SPEC.md §6.3, UI API 1.5): rendered headless at each width
//! class, the path-based `line` and `pie` kinds (and the `bar` kind) must
//! actually paint colored geometry inside their `Figure`, and keep the
//! hidden AccessKit summary of every value. Writes
//! `target/shots/chart-{compact,regular,wide}.png` for a visual check.

use gpui::{AppContext as _, HeadlessAppContext, PlatformHeadlessRenderer, px, size};
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
import { app, Screen, Section, Chart } from \"plinth:ui\";

function Home() {
  return (
    <Screen title=\"Charts\">
      <Section title=\"Line\">
        <Chart
          label=\"Monthly spending\"
          kind=\"line\"
          data={[
            { label: \"Jan\", value: 420 },
            { label: \"Feb\", value: 380 },
            { label: \"Mar\", value: 610 },
            { label: \"Apr\", value: 540 },
            { label: \"May\", value: 720 },
            { label: \"Jun\", value: 655 },
          ]}
          series={[
            { name: \"2025\", points: [
              { label: \"Jan\", value: 420 }, { label: \"Feb\", value: 380 }, { label: \"Mar\", value: 610 },
              { label: \"Apr\", value: 540 }, { label: \"May\", value: 720 }, { label: \"Jun\", value: 655 },
            ] },
            { name: \"2026\", points: [
              { label: \"Jan\", value: 300 }, { label: \"Feb\", value: 450 }, { label: \"Mar\", value: 500 },
              { label: \"Apr\", value: 680 }, { label: \"May\", value: 590 }, { label: \"Jun\", value: 810 },
            ] },
          ]}
        />
      </Section>
      <Section title=\"Pie\">
        <Chart
          label=\"Spending share\"
          kind=\"pie\"
          data={[
            { label: \"Rent\", value: 1200 },
            { label: \"Groceries\", value: 450 },
            { label: \"Transport\", value: 160 },
            { label: \"Fun\", value: 220 },
            { label: \"Other\", value: 90 },
          ]}
        />
      </Section>
      <Section title=\"Bar\">
        <Chart
          label=\"By category\"
          kind=\"bar\"
          data={[
            { label: \"Rent\", value: 1200 },
            { label: \"Groceries\", value: 450 },
            { label: \"Fun\", value: 220 },
          ]}
        />
        <Chart
          label=\"Net by month\"
          kind=\"bar\"
          data={[
            { label: \"Jan\", value: 300 },
            { label: \"Feb\", value: -150 },
            { label: \"Mar\", value: 0 },
            { label: \"Apr\", value: 200 },
          ]}
        />
      </Section>
    </Screen>
  );
}

export default app({ screens: { home: { title: \"Charts\", component: Home } } });
";

const SIZES: [(&str, f32, f32); 3] = [("compact", 400., 1500.), ("regular", 900., 1300.), ("wide", 1280., 1300.)];

fn build() -> plinth_compiler::Artifact {
    let fs = MemFs::default().with("app/main.tsx", APP);
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[]).expect("compile");
    let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
    artifact.unwrap_or_else(|| panic!("chart test app has errors:\n{}", diags.join("\n")))
}

/// The top and bottom rows of clearly colored pixels in the x range
/// `x0..x1` (the middle of one bar column) between `y0` and `y1`.
fn colored_rows(rgba: &[u8], (w, _h): (u32, u32), x0: u32, x1: u32, y0: u32, y1: u32) -> Option<(u32, u32)> {
    let mut rows = (y0..y1).filter(|&y| {
        (x0..x1).any(|x| {
            let i = ((y * w + x) * 4) as usize;
            let (r, g, b) = (rgba[i], rgba[i + 1], rgba[i + 2]);
            r.max(g).max(b) - r.min(g).min(b) > 60
        })
    });
    let first = rows.next()?;
    Some((first, rows.last().unwrap_or(first)))
}

/// The share of pixels inside `r` that are clearly colored (the chart
/// palette), as opposed to the neutral surface, text and gridlines.
fn colored_share(rgba: &[u8], (w, h): (u32, u32), r: gpui::accesskit::Rect) -> f64 {
    let (x0, y0) = (r.x0.max(0.0) as u32, r.y0.max(0.0) as u32);
    let (x1, y1) = ((r.x1 as u32).min(w), (r.y1 as u32).min(h));
    let (mut colored, mut total) = (0u64, 0u64);
    for y in y0..y1 {
        for x in x0..x1 {
            let i = ((y * w + x) * 4) as usize;
            let (r, g, b) = (rgba[i], rgba[i + 1], rgba[i + 2]);
            let (hi, lo) = (r.max(g).max(b), r.min(g).min(b));
            total += 1;
            if hi - lo > 60 {
                colored += 1;
            }
        }
    }
    colored as f64 / total.max(1) as f64
}

#[test]
fn line_and_pie_charts_paint_geometry_at_every_width() {
    let artifact = build();

    let platform = gpui_platform::current_platform(true);
    let mut cx = HeadlessAppContext::with_platform(platform.text_system(), std::sync::Arc::new(()), || {
        gpui_wgpu::WgpuHeadlessRenderer::new()
            .map(|r| Box::new(r) as Box<dyn PlatformHeadlessRenderer>)
            .map_err(|e| log::error!("no headless renderer: {e:#}"))
            .ok()
    });
    cx.update(plinth_ui::init);

    let shots = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/shots");
    std::fs::create_dir_all(&shots).unwrap();

    for (class, w, h) in SIZES {
        let runner = Runner::new().unwrap();
        let mut guest = runner.load(&artifact.component, Limits::default()).unwrap();
        let commits = guest.init(&[]).unwrap();
        let port = Box::new(WasmGuest { guest, _runner: runner });
        let window =
            cx.open_window(size(px(w), px(h)), move |_, cx| cx.new(|cx| PlinthRoot::new(port, commits, "teal", cx))).unwrap();
        cx.update(|cx| window.update(cx, |_, window, _| window.set_a11y_forced(true))).unwrap();
        cx.run_until_parked();

        let tree = cx.update(|cx| window.update(cx, |_, window, _| window.a11y_tree().cloned())).unwrap().unwrap();
        let found: Vec<_> = tree
            .nodes
            .iter()
            .map(|(_, n)| (n.role(), n.label().or(n.value()).unwrap_or("").to_owned(), n.bounds()))
            .collect();
        let figure = |name: &str| {
            found
                .iter()
                .find(|(r, l, _)| *r == gpui::accesskit::Role::Figure && l == name)
                .and_then(|(_, _, b)| *b)
                .unwrap_or_else(|| panic!("[{class}] no Figure named {name:?}: {found:#?}"))
        };
        let described = |text: &str| found.iter().any(|(r, l, _)| *r == gpui::accesskit::Role::Label && l.contains(text));

        // Accessibility: every value is still in the hidden summary.
        assert!(described("2025: Jan: 420, Feb: 380"), "[{class}] line summary missing: {found:#?}");
        assert!(described("2026: Jan: 300"), "[{class}] second series summary missing: {found:#?}");
        assert!(described("Rent: 1200, Groceries: 450, Transport: 160, Fun: 220, Other: 90"), "[{class}] pie summary missing");
        assert!(described("Rent: 1200, Groceries: 450, Fun: 220"), "[{class}] bar summary missing");
        assert!(described("Jan: 300, Feb: -150, Mar: 0, Apr: 200"), "[{class}] negative bar summary missing");

        let image = cx.capture_screenshot(window.into()).expect("render the charts");
        image.save(shots.join(format!("chart-{class}.png"))).expect("write the chart screenshot");

        // Geometry: the pie is mostly filled wedges, the lines are thin
        // strokes but clearly present, the bars unchanged.
        let pie = colored_share(image.as_raw(), image.dimensions(), figure("Spending share"));
        let line = colored_share(image.as_raw(), image.dimensions(), figure("Monthly spending"));
        let bar = colored_share(image.as_raw(), image.dimensions(), figure("By category"));
        assert!(pie > 0.15, "[{class}] pie paints too little color: {pie:.3}");
        assert!(line > 0.01 && line < 0.4, "[{class}] line paints an unexpected amount of color: {line:.3}");
        assert!(bar > 0.05, "[{class}] bars paint too little color: {bar:.3}");

        // Negative values: a bar grows down from the zero line, where the
        // positive bars end; a zero value has no bar (not even a stub).
        let net = figure("Net by month");
        let col = |i: u32| {
            let cw = (net.x1 - net.x0) / 4.0;
            let x = net.x0 + cw * i as f64;
            colored_rows(image.as_raw(), image.dimensions(), (x + cw * 0.3) as u32, (x + cw * 0.5) as u32, net.y0 as u32, net.y1 as u32)
        };
        let (jan_top, jan_bottom) = col(0).unwrap_or_else(|| panic!("[{class}] no Jan bar"));
        let (feb_top, feb_bottom) = col(1).unwrap_or_else(|| panic!("[{class}] no Feb bar"));
        assert!(col(2).is_none(), "[{class}] a zero value paints a bar: {:?}", col(2));
        assert!(feb_top + 3 >= jan_bottom, "[{class}] the negative bar does not start at the zero line: Jan {jan_top}..{jan_bottom}, Feb {feb_top}..{feb_bottom}");
        assert!(feb_bottom > jan_bottom + 10, "[{class}] the negative bar does not go below zero: Jan {jan_top}..{jan_bottom}, Feb {feb_top}..{feb_bottom}");
    }
}
