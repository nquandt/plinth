//! The desktop host (SPEC.md §9): it loads an app component, runs it in
//! wasmtime and renders it with gpui-ce. `plinth dev` and `plinth run` use
//! it in-process; `plinth-host` is a thin binary around it.

use anyhow::{Context as _, Result};
use gpui::{App, AppContext, Bounds, KeyBinding, WindowBounds, WindowOptions, actions, px, size};
use plinth_runner_wasmtime::{Clipboard, Guest, Limits, Runner, kv::Kv, policy::Policy};
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
    /// The manifest id, used to isolate `store.kv` data per app (SPEC.md
    /// §11). A bare `app.wasm` with no manifest gets a `dev.`-prefixed id
    /// derived from the title.
    pub app_id: String,
    /// The capability names the manifest (or `plinth.toml`) declares
    /// (SPEC.md §11). A bare `app.wasm` with no manifest gets none.
    pub capabilities: Vec<String>,
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
                app_id: pkg.manifest.id,
                capabilities: pkg.manifest.capabilities.into_iter().map(|c| c.name).collect(),
            })
        } else {
            let title = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            Ok(HostApp { component: bytes, title: title.clone(), accent: "teal".into(), app_id: format!("dev.{title}"), capabilities: Vec::new() })
        }
    }
}

/// A `Clipboard` backed by the real system clipboard (`arboard`), used
/// when a `Policy` grants `clipboard.read`/`clipboard.write` (SPEC.md
/// §9.4, §11). Each call opens the platform clipboard fresh; it is not
/// kept open between calls, so other apps may use it meanwhile.
#[derive(Default)]
struct SystemClipboard;

impl Clipboard for SystemClipboard {
    fn write_text(&mut self, text: &str) {
        match arboard::Clipboard::new() {
            Ok(mut cb) => {
                if let Err(e) = cb.set_text(text) {
                    log::warn!("clipboard write failed: {e}");
                }
            }
            Err(e) => log::warn!("clipboard unavailable: {e}"),
        }
    }

    fn read_text(&mut self) -> Option<String> {
        match arboard::Clipboard::new() {
            Ok(mut cb) => cb.get_text().ok(),
            Err(e) => {
                log::warn!("clipboard unavailable: {e}");
                None
            }
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

    fn next_timer_deadline(&self) -> Option<std::time::Instant> {
        self.guest.next_timer_deadline()
    }

    fn fire_due_timers(&mut self, now: std::time::Instant) -> Result<Vec<Vec<u8>>> {
        let r = self.guest.fire_due_timers(now);
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

/// Instantiates a component and runs `init`. Builds the capability policy
/// from `capabilities` (SPEC.md §11: a declared capability is granted;
/// there is no consent UI yet, SPEC.md §11 stretch) and opens the
/// `store.kv` file for `app_id`.
fn start(component: &[u8], app_id: &str, capabilities: &[String], args: &[u8]) -> (Box<dyn GuestPort>, Result<Vec<Vec<u8>>, String>) {
    let policy = Policy::new(capabilities.iter().cloned());
    let kv = Kv::open(&plinth_runner_wasmtime::kv::data_dir(), app_id).unwrap_or_else(|e| {
        log::warn!("store.kv unavailable for {app_id}: {e:#}");
        Kv::in_memory()
    });
    let clipboard: Box<dyn Clipboard> = Box::new(SystemClipboard);
    let loaded = Runner::new().and_then(|runner| {
        let guest = runner.load_with_policy(component, Limits::default(), policy, kv, clipboard)?;
        Ok((runner, guest))
    });
    match loaded {
        Ok((runner, mut guest)) => {
            // Hot reload (SPEC.md §13): `args` is the previous instance's
            // signal snapshot, or empty on the first start. A release
            // build of the app ignores it (it never registered anything).
            let init = guest.init(args).map_err(|e| format!("{e:#}"));
            print_logs(&mut guest);
            (Box::new(WasmGuest { guest, _runner: runner }), init)
        }
        Err(e) => (Box::new(NoGuest), Err(format!("{e:#}"))),
    }
}

/// Opens the app window and runs until it closes. Each component that
/// arrives on `reloads` replaces the running app (hot reload).
pub fn run(app: HostApp, reloads: Option<Receiver<Vec<u8>>>) -> Result<()> {
    let (port, init) = start(&app.component, &app.app_id, &app.capabilities, &[]);
    let HostApp { title, accent, app_id, capabilities, .. } = app;

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
            let app_id = app_id.clone();
            let capabilities = capabilities.clone();
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
                        let ok = window
                            .update(cx, |root, _, cx| {
                                root.reload(|args| start(&bytes, &app_id, &capabilities, args), cx)
                            })
                            .is_ok();
                        if !ok {
                            return;
                        }
                        eprintln!("[plinth] reloaded");
                    }
                }
            })
            .detach();
        }

        // Drives `plinth:time` timers (SPEC.md §8.4, §8.5, §9.4): a short
        // fixed tick is simpler and robust enough than sleeping until the
        // next exact deadline, and `poll_timers` is a cheap no-op when
        // nothing is due.
        cx.spawn(async move |cx| loop {
            cx.background_executor().timer(Duration::from_millis(15)).await;
            if window.update(cx, |root, _, cx| root.poll_timers(cx)).is_err() {
                return;
            }
        })
        .detach();
    });
    Ok(())
}

/// Installs the default logger (`RUST_LOG` overrides the level).
pub fn init_logging() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).try_init();
}
