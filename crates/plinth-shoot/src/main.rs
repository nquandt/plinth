//! `plinth-shoot <app.plnt|app.wasm> <out-dir>`: renders each screen of an
//! app at each width class to PNG files, without a window (SPEC.md §13,
//! `plinth shoot`). It uses the headless WGPU renderer of gpui-ce.

use anyhow::{Context as _, Result, bail};
use gpui::{AppContext as _, HeadlessAppContext, PlatformHeadlessRenderer, px, size};
use plinth_host_desktop::HostApp;
use plinth_runner_wasmtime::{Guest, Limits, Runner};
use plinth_ui::{GuestPort, PlinthRoot};
use std::path::PathBuf;
use std::sync::Arc;

/// One window size for each width class (SPEC.md §6.1).
const SIZES: [(&str, f32, f32); 3] = [("compact", 400., 760.), ("regular", 900., 760.), ("wide", 1280., 800.)];

struct WasmGuest {
    guest: Guest,
    _runner: Runner,
}

impl GuestPort for WasmGuest {
    fn dispatch(&mut self, events: &[u8]) -> Result<Vec<Vec<u8>>> {
        self.guest.on_event(events)
    }
}

fn main() -> Result<()> {
    plinth_host_desktop::init_logging();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, out] = args.as_slice() else {
        bail!("usage: plinth-shoot <app.plnt|app.wasm> <out-dir>");
    };
    let app = HostApp::load(input.as_ref())?;
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).with_context(|| format!("create {}", out.display()))?;

    let platform = gpui_platform::current_platform(true);
    let mut cx = HeadlessAppContext::with_platform(platform.text_system(), Arc::new(()), || {
        gpui_wgpu::WgpuHeadlessRenderer::new()
            .map(|r| Box::new(r) as Box<dyn PlatformHeadlessRenderer>)
            .map_err(|e| log::error!("no headless renderer: {e:#}"))
            .ok()
    });
    cx.update(plinth_ui::init);

    let mut count = 0;
    for (class, w, h) in SIZES {
        // A new guest for each size, so each shot shows the initial state.
        let runner = Runner::new()?;
        let mut guest = runner.load(&app.component, Limits::default())?;
        let commits = guest.init(&[])?;
        let port = Box::new(WasmGuest { guest, _runner: runner });
        let accent = app.accent.clone();
        let window = cx.open_window(size(px(w), px(h)), move |_, cx| cx.new(|cx| PlinthRoot::new(port, commits, accent, cx)))?;
        let screens: Vec<u32> =
            cx.update_window(window.into(), |_, _, cx| window.read(cx).map(|r| r.tree().screens().map(|(s, _)| s).collect()))??;
        for screen in screens {
            cx.update_window(window.into(), |_, _, cx| window.update(cx, |root, _, cx| root.select_screen(screen, cx)))??;
            cx.run_until_parked();
            let image = cx.capture_screenshot(window.into()).context("render the screen")?;
            let path = out.join(format!("screen{screen}-{class}.png"));
            image.save(&path).with_context(|| format!("write {}", path.display()))?;
            println!("{}", path.display());
            count += 1;
        }
    }
    eprintln!("[plinth] {count} screenshots in {}", out.display());
    Ok(())
}
