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

pub mod consent;

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
    /// The package's assets (SPEC.md §10.1), by path under `assets/`
    /// without the prefix, for `<Image>`. A bare `app.wasm` has none; the
    /// `plinth dev` host adds the project's `assets/` itself.
    pub assets: std::collections::HashMap<String, Vec<u8>>,
}

impl HostApp {
    /// Reads a `.plnt` package, a bare app module or a bare component. An
    /// app module holds only the app code; the host links it into its own
    /// runtime (SPEC.md §10.4).
    pub fn load(path: &Path) -> Result<HostApp> {
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        Self::from_bytes(bytes, path)
    }

    /// Like `load`, for bytes that are already in memory (for example the
    /// payload of a single-file export). `path` names them in messages.
    pub fn from_bytes(bytes: Vec<u8>, path: &Path) -> Result<HostApp> {
        if plinth_package::is_package(&bytes) {
            let pkg = plinth_package::Package::read(&bytes).with_context(|| format!("open {}", path.display()))?;
            let assets = pkg.assets.into_iter().filter_map(|(p, b)| Some((p.strip_prefix("assets/")?.to_owned(), b))).collect();
            let capabilities: Vec<String> = pkg.manifest.capabilities.into_iter().map(|c| c.name).collect();
            Ok(HostApp {
                component: with_runtime(pkg.component, &capabilities).with_context(|| format!("load {}", path.display()))?,
                title: pkg.manifest.name,
                accent: pkg.manifest.accent.unwrap_or_else(|| "teal".into()),
                app_id: pkg.manifest.id,
                capabilities,
                assets,
            })
        } else {
            let title = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let component = with_runtime(bytes, &[]).with_context(|| format!("load {}", path.display()))?;
            Ok(HostApp {
                component,
                title: title.clone(),
                accent: "teal".into(),
                app_id: format!("dev.{title}"),
                capabilities: Vec::new(),
                assets: std::collections::HashMap::new(),
            })
        }
    }
}

/// Links an app module into the installed core that it needs (SPEC.md
/// §10.5). A component passes through unchanged. Refuses an app module
/// that can reach a capability its manifest does not declare (`SPEC.md`
/// §11, `docs/HUB.md` §7.1 rule 1); `declared` is empty for a bare
/// `app.wasm`, so it may reach none.
fn with_runtime(entry: Vec<u8>, declared: &[String]) -> Result<Vec<u8>> {
    use plinth_link::{cores, split};
    if split::is_app_module(&entry) {
        split::check_capabilities(&entry, declared)?;
        cores::link_app(&entry)
    } else {
        Ok(entry)
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

    fn pending_dialogs(&self) -> Vec<plinth_ui::PendingDialog> {
        self.guest
            .pending_dialogs()
            .iter()
            .map(|d| plinth_ui::PendingDialog {
                id: d.id,
                kind: match d.kind {
                    plinth_runner_wasmtime::DialogKind::Alert => plinth_ui::DialogKind::Alert,
                    plinth_runner_wasmtime::DialogKind::Confirm => plinth_ui::DialogKind::Confirm,
                    plinth_runner_wasmtime::DialogKind::Prompt => plinth_ui::DialogKind::Prompt,
                },
                message: d.message.clone(),
            })
            .collect()
    }

    fn answer_dialog(&mut self, id: u32, value: plinth_protocol::Value) -> Result<Vec<Vec<u8>>> {
        let r = self.guest.answer_dialog(id, value);
        print_logs(&mut self.guest);
        r
    }

    fn poll_net_results(&mut self) -> Vec<(u32, plinth_protocol::Value)> {
        self.guest.poll_net_results()
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
    start_with_policy(component, app_id, policy, args)
}

/// Like `start`, but with a policy the caller already built (for example
/// from the Hub's grants store, `docs/HUB.md` §7, §12.4, phase H0 part 2),
/// instead of "declared is granted".
pub fn start_with_policy(component: &[u8], app_id: &str, policy: Policy, args: &[u8]) -> (Box<dyn GuestPort>, Result<Vec<Vec<u8>>, String>) {
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
            let init_args = if args.is_empty() {
                Vec::new()
            } else {
                plinth_protocol::init_arg::one(plinth_protocol::init_arg::SNAPSHOT, args)
            };
            let init = guest.init(&init_args).map_err(|e| format!("{e:#}"));
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
    let HostApp { title, accent, app_id, capabilities, assets, .. } = app;
    let assets = std::sync::Arc::new(assets);

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
                    Ok(commits) => PlinthRoot::with_assets(port, commits, accent, assets, cx),
                    Err(e) => PlinthRoot::stopped(port, e, accent, cx),
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

/// Runs one library app from the Hub (`docs/HUB.md` §7.3, §9, phase H0
/// parts 2–3): refuses a blocked app, shows the consent screen for any
/// declared capability that has no grant yet, saves the decisions, then
/// opens the app with a policy built from the grants (declared AND
/// allowed is granted; declared and refused is denied). Returns `Ok(())`
/// without running anything if the user cancels consent.
pub fn run_from_hub(hub: &plinth_hub::Hub, app_id: &str) -> Result<()> {
    if hub.is_blocked(app_id)? {
        anyhow::bail!("{app_id} is blocked; it will not run");
    }
    let bytes = hub.package(app_id)?;
    let pkg = plinth_package::Package::read(&bytes).with_context(|| format!("read the package for {app_id}"))?;
    let declared: Vec<String> = pkg.manifest.capabilities.iter().map(|c| c.name.clone()).collect();

    let pending = hub.needs_consent(app_id, &declared)?;
    if !pending.is_empty() {
        let publisher = pkg.manifest.publisher.clone();
        let with_reasons: Vec<(String, String)> = pending
            .iter()
            .map(|c| {
                let why = pkg.manifest.capabilities.iter().find(|m| &m.name == c).map(|m| m.rationale.clone()).unwrap_or_default();
                (c.clone(), why)
            })
            .collect();
        let Some(decisions) = consent::show(&pkg.manifest.name, &publisher, &with_reasons) else {
            eprintln!("[plinth] consent cancelled; {app_id} will not run");
            return Ok(());
        };
        for (capability, allowed) in decisions {
            let decision = if allowed { plinth_hub::Decision::Allowed } else { plinth_hub::Decision::Refused };
            hub.set_grant(app_id, &capability, decision, &pkg.manifest.version)?;
        }
    }

    let policy = hub.policy_for(app_id, &declared)?;
    let component = with_runtime(pkg.component, &declared)?;
    let assets: std::collections::HashMap<String, Vec<u8>> =
        pkg.assets.into_iter().filter_map(|(p, b)| Some((p.strip_prefix("assets/")?.to_owned(), b))).collect();
    run_with_policy(component, pkg.manifest.name, pkg.manifest.accent.unwrap_or_else(|| "teal".into()), app_id.to_owned(), policy, assets)
}

/// Opens the app window with a policy the caller already built (`Policy`
/// from grants, instead of "declared is granted"). No hot reload: this
/// path is for library apps, not `plinth dev`.
fn run_with_policy(
    component: Vec<u8>,
    title: String,
    accent: String,
    app_id: String,
    policy: Policy,
    assets: std::collections::HashMap<String, Vec<u8>>,
) -> Result<()> {
    let (port, init) = start_with_policy(&component, &app_id, policy, &[]);
    let assets = std::sync::Arc::new(assets);

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
            .titlebar(Some(gpui::TitlebarOptions { title: Some(title.clone().into()), ..Default::default() }));
        cx.open_window(options, move |_, cx| {
            cx.new(move |cx| match init {
                Ok(commits) => PlinthRoot::with_assets(port, commits, accent, assets, cx),
                Err(e) => PlinthRoot::stopped(port, e, accent, cx),
            })
        })
        .expect("open the window");
        cx.activate(true);
    });
    Ok(())
}

/// Installs the default logger (`RUST_LOG` overrides the level).
pub fn init_logging() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).try_init();
}
