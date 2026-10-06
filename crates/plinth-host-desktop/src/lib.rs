//! The desktop host (SPEC.md §9): it loads an app component, runs it in
//! wasmtime and renders it with gpui-ce. `plinth dev` and `plinth run` use
//! it in-process; `plinth-host` is a thin binary around it.

use anyhow::{Context as _, Result};
use gpui::{App, AppContext, Bounds, KeyBinding, WindowBounds, WindowOptions, actions, px, size};
use plinth_runner_wasmtime::{Guest, Limits, Runner};
use plinth_ui::{GuestPort, PlinthRoot};
use std::path::Path;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

actions!(plinth_host, [Quit]);

/// An app that is ready to run.
pub struct HostApp {
    pub component: Vec<u8>,
    pub title: String,
    pub accent: String,
}

impl HostApp {
    /// Reads a `.plnt` package or a bare `app.wasm` component.
    pub fn load(path: &Path) -> Result<HostApp> {
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        if plinth_package::is_package(&bytes) {
            let pkg = plinth_package::Package::read(&bytes).with_context(|| format!("open {}", path.display()))?;
            Ok(HostApp {
                component: pkg.component,
                title: pkg.manifest.name,
                accent: pkg.manifest.accent.unwrap_or_else(|| "teal".into()),
            })
        } else {
            let title = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            Ok(HostApp { component: bytes, title, accent: "teal".into() })
        }
    }
}

struct WasmGuest {
    guest: Guest,
    // The runner owns the engine's epoch ticker, so it lives as long as the guest.
    _runner: Runner,
}

impl GuestPort for WasmGuest {
    fn dispatch(&mut self, events: &[u8]) -> Result<Vec<Vec<u8>>> {
        let r = self.guest.on_event(events);
        print_logs(&mut self.guest);
        r
    }
}

fn print_logs(guest: &mut Guest) {
    for line in guest.take_logs() {
        eprintln!("[app] {line}");
    }
}

/// A placeholder guest for an artifact that failed to load.
struct NoGuest;

impl GuestPort for NoGuest {
    fn dispatch(&mut self, _: &[u8]) -> Result<Vec<Vec<u8>>> {
        anyhow::bail!("the app is not running")
    }
}

/// Instantiates a component and runs `init`.
fn start(component: &[u8]) -> (Box<dyn GuestPort>, Result<Vec<Vec<u8>>, String>) {
    let loaded = Runner::new().and_then(|runner| {
        let guest = runner.load(component, Limits::default())?;
        Ok((runner, guest))
    });
    match loaded {
        Ok((runner, mut guest)) => {
            let init = guest.init(&[]).map_err(|e| format!("{e:#}"));
            print_logs(&mut guest);
            (Box::new(WasmGuest { guest, _runner: runner }), init)
        }
        Err(e) => (Box::new(NoGuest), Err(format!("{e:#}"))),
    }
}

/// Opens the app window and runs until it closes. Each component that
/// arrives on `reloads` replaces the running app (hot reload).
pub fn run(app: HostApp, reloads: Option<Receiver<Vec<u8>>>) -> Result<()> {
    let (port, init) = start(&app.component);
    let HostApp { title, accent, .. } = app;

    gpui_platform::application().run(move |cx: &mut App| {
        plinth_ui::init(cx);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(1000.), px(720.)), cx);
        let options = WindowOptions::new()
            .window_bounds(Some(WindowBounds::Windowed(bounds)))
            .titlebar(Some(gpui::TitlebarOptions { title: Some(title.into()), ..Default::default() }));
        let window = cx
            .open_window(options, move |_, cx| {
                cx.new(move |cx| match init {
                    Ok(commits) => PlinthRoot::new(port, commits, accent, cx),
                    Err(e) => PlinthRoot::stopped(port, e, accent),
                })
            })
            .expect("open the window");
        cx.activate(true);

        if let Some(rx) = reloads {
            cx.spawn(async move |cx| {
                loop {
                    cx.background_executor().timer(Duration::from_millis(100)).await;
                    let mut latest = None;
                    loop {
                        match rx.try_recv() {
                            Ok(bytes) => latest = Some(bytes),
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => return,
                        }
                    }
                    if let Some(bytes) = latest {
                        let (port, init) = start(&bytes);
                        let ok = window.update(cx, |root, _, cx| root.reload(port, init, cx)).is_ok();
                        if !ok {
                            return;
                        }
                        eprintln!("[plinth] reloaded");
                    }
                }
            })
            .detach();
        }
    });
    Ok(())
}

/// Installs the default logger (`RUST_LOG` overrides the level).
pub fn init_logging() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).try_init();
}
