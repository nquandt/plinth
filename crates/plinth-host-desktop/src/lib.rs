//! The desktop host (SPEC.md §9): it loads an app component, runs it in
//! wasmtime and renders it with gpui-ce. `plinth dev` and `plinth run` use
//! it in-process; `plinth-host` is a thin binary around it.

use anyhow::{Context as _, Result};
use gpui::{App, AppContext, Bounds, KeyBinding, WindowBounds, WindowOptions, actions, px, size};
use plinth_runner_wasmtime::{Clipboard, Guest, Limits, Runner, kv::Kv, policy::Policy};
use plinth_ui::{GuestPort, PlinthRoot};
use std::path::Path;
use std::sync::Arc;
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
            let capabilities: Vec<String> = pkg.manifest.capabilities.iter().map(|c| c.name.clone()).collect();
            check_hub_trust(&pkg, &capabilities).with_context(|| format!("load {}", path.display()))?;
            let assets = pkg.assets.into_iter().filter_map(|(p, b)| Some((p.strip_prefix("assets/")?.to_owned(), b))).collect();
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

/// A future key id for the Plinth project itself (`docs/HUB.md` §4.1,
/// §12.3): a package signed by it is trusted for `hub.manage` on every
/// host, with no `PLINTH_HUB_TRUSTED_KEYS` setting needed. Left empty
/// until that key exists; an empty string never matches a real signer.
const PLINTH_PROJECT_HUB_KEY: &str = "";

/// Checks the trusted-signer rule for `hub.manage` (`docs/HUB.md` §4.1):
/// the host grants `hub.manage` only to a package signed by a key it
/// trusts as a Hub key — for now, a key id listed in the comma-separated
/// `PLINTH_HUB_TRUSTED_KEYS` environment variable, or the (currently
/// empty) Plinth project key. A package that declares `hub.manage` but is
/// not signed by a trusted key is refused outright, with a clear error,
/// before it ever runs; a package that does not declare `hub.manage` is
/// unaffected (this is the only capability with an extra, signer-based
/// rule — every other capability only needs the manifest and consent).
fn check_hub_trust(pkg: &plinth_package::Package, declared: &[String]) -> Result<()> {
    if !declared.iter().any(|c| c == plinth_runner_wasmtime::capability::HUB_MANAGE) {
        return Ok(());
    }
    let signer = plinth_package::signature::verify(pkg).context("the package signature does not check out")?;
    let key = signer.map(|s| s.key).unwrap_or_default();
    let trusted = key_is_trusted(&key);
    if trusted {
        Ok(())
    } else if key.is_empty() {
        anyhow::bail!(
            "{} declares `hub.manage` but is unsigned; only a package signed by a trusted Hub key may use `plinth:hub` (docs/HUB.md §4.1)",
            pkg.manifest.id
        )
    } else {
        anyhow::bail!(
            "{} declares `hub.manage` but is signed by `{key}`, which this host does not trust as a Hub key (set PLINTH_HUB_TRUSTED_KEYS, docs/HUB.md §4.1)",
            pkg.manifest.id
        )
    }
}

/// Whether `key` (a signer's key id, for example `ed25519:…`) is one this
/// host trusts for `hub.manage`.
fn key_is_trusted(key: &str) -> bool {
    if key.is_empty() {
        return false;
    }
    if !PLINTH_PROJECT_HUB_KEY.is_empty() && key == PLINTH_PROJECT_HUB_KEY {
        return true;
    }
    std::env::var("PLINTH_HUB_TRUSTED_KEYS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .any(|trusted| !trusted.is_empty() && trusted == key)
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
    // The runner owns the engine's epoch ticker; several guests across
    // several windows share one runner (`docs/HUB.md` §4.2, §12.2), so it
    // is kept alive by reference count rather than owned here.
    _runner: Arc<Runner>,
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

    fn take_hub_launches(&mut self) -> Vec<String> {
        self.guest.take_hub_launches()
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

/// Instantiates a component and runs `init`, with a policy the caller
/// already built (for example from the Hub's grants store, `docs/HUB.md`
/// §7, §12.4, phase H0 part 2) instead of "declared is granted", and a
/// `Runner` the caller already has, shared across every app the process
/// runs (`docs/HUB.md` §4.2, §12.2).
pub fn start_with_policy(
    runner: Arc<Runner>,
    component: &[u8],
    app_id: &str,
    policy: Policy,
    args: &[u8],
) -> (Box<dyn GuestPort>, Result<Vec<Vec<u8>>, String>) {
    start_with_policy_and_hub(runner, component, app_id, policy, None, args)
}

/// Like `start_with_policy`, with a `plinth:hub` backend (`docs/HUB.md`
/// §4.1, §12.2) for a guest the host trusted with `hub.manage`
/// (`check_hub_trust` must already have passed before this is called).
pub fn start_with_policy_and_hub(
    runner: Arc<Runner>,
    component: &[u8],
    app_id: &str,
    policy: Policy,
    hub: Option<Box<dyn plinth_runner_wasmtime::hub::HubBackend>>,
    args: &[u8],
) -> (Box<dyn GuestPort>, Result<Vec<Vec<u8>>, String>) {
    let kv = Kv::open(&plinth_runner_wasmtime::kv::data_dir(), app_id).unwrap_or_else(|e| {
        log::warn!("store.kv unavailable for {app_id}: {e:#}");
        Kv::in_memory()
    });
    let clipboard: Box<dyn Clipboard> = Box::new(SystemClipboard);
    let loaded = runner.load_with_policy_and_hub(component, Limits::default(), policy, kv, clipboard, hub);
    match loaded {
        Ok(mut guest) => {
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

/// Registers the app-level behavior that every host window shares: `Cmd/
/// Ctrl+Q` quits, and the process exits once the last window closes
/// (`docs/HUB.md` §4.2: closing one app window must not close the others,
/// so this is set up once per `gpui::App`, not once per window). Callers
/// that open more than one window (the Hub, later) call this one time
/// before the first `open_app`.
pub fn init_app(cx: &mut App) {
    plinth_ui::init(cx);
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
}

/// Opens one app window: its own guest instance (on `runner`, which may be
/// shared with other open app windows, `docs/HUB.md` §4.2, §12.2), its own
/// policy, data directory (the `store.kv` file is keyed by `app.app_id`),
/// assets and `PlinthRoot`. Closing this window does not close any other;
/// the caller's `gpui::App` already arranged (`init_app`) for the process
/// to exit when the last window closes. Any code, including the future
/// Hub UI, can call this to launch an app.
pub fn open_app(cx: &mut App, runner: Arc<Runner>, app: HostApp, policy: Policy) -> gpui::WindowHandle<PlinthRoot> {
    open_app_with_hub(cx, runner, app, policy, None)
}

/// Like `open_app`, with a `plinth:hub` backend (`docs/HUB.md` §4.1,
/// §12.2) for a guest the host trusted with `hub.manage`.
pub fn open_app_with_hub(
    cx: &mut App,
    runner: Arc<Runner>,
    app: HostApp,
    policy: Policy,
    hub: Option<Box<dyn plinth_runner_wasmtime::hub::HubBackend>>,
) -> gpui::WindowHandle<PlinthRoot> {
    let HostApp { component, title, accent, app_id, assets, .. } = app;
    let (port, init) = start_with_policy_and_hub(runner, &component, &app_id, policy, hub, &[]);
    let assets = Arc::new(assets);

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

    // Drives this window's `plinth:time` timers (SPEC.md §8.4, §8.5,
    // §9.4). Each app window polls its own guest independently
    // (`docs/HUB.md` §4.2): one app's timers never touch another's.
    cx.spawn(async move |cx| loop {
        cx.background_executor().timer(Duration::from_millis(15)).await;
        if window.update(cx, |root, _, cx| root.poll_timers(cx)).is_err() {
            return;
        }
    })
    .detach();

    window
}

/// Opens the app window and runs until the process's last window closes.
/// Each component that arrives on `reloads` replaces the running app in
/// that same window (hot reload); `plinth dev`/`plinth run` only ever open
/// one window, but the window closing no longer quits other windows a
/// later caller (the Hub) may have opened in the same process.
pub fn run(app: HostApp, reloads: Option<Receiver<Vec<u8>>>) -> Result<()> {
    let runner = Arc::new(Runner::new()?);
    let app_id = app.app_id.clone();
    let capabilities = app.capabilities.clone();
    let policy = Policy::new(capabilities.iter().cloned());

    gpui_platform::application().run(move |cx: &mut App| {
        init_app(cx);
        let window = open_app(cx, runner.clone(), app, policy);
        cx.activate(true);

        if let Some(rx) = reloads {
            let runner = runner.clone();
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
                        let runner = runner.clone();
                        let app_id = app_id.clone();
                        let capabilities = capabilities.clone();
                        let ok = window
                            .update(cx, |root, _, cx| {
                                root.reload(
                                    |args| {
                                        let policy = Policy::new(capabilities.iter().cloned());
                                        start_with_policy(runner, &bytes, &app_id, policy, args)
                                    },
                                    cx,
                                )
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
    });
    Ok(())
}

// -- The Hub host (`docs/HUB.md` §4.2, §7.3, phase H3) ----------------------

/// A library app that is ready to open: its component is linked, its
/// policy comes from the grants, and the trusted-signer rule for
/// `hub.manage` passed.
pub struct PreparedApp {
    pub app: HostApp,
    pub policy: Policy,
    /// `true` when the app declares `hub.manage` (the Hub UI): it gets a
    /// `plinth:hub` backend, and the host polls it for launch requests.
    pub hub_manage: bool,
}

/// What the host must do to open a library app (`docs/HUB.md` §7.3).
pub enum LaunchPlan {
    /// Every declared capability is decided: open the app.
    Ready(Box<PreparedApp>),
    /// Show the consent window for `pending` (capability, reason of the
    /// app) first, then call `apply_consent` with the decisions.
    NeedsConsent { app_name: String, publisher: String, signer: Option<String>, version: String, pending: Vec<(String, String)> },
}

/// Decides how to open `app_id` from the library: refuses a blocked app,
/// asks for consent when the pinned-or-latest version declares a
/// capability that has no decision (`docs/HUB.md` §7.3 step 3), else
/// prepares the app. No window opens here, so a test can call it.
pub fn plan_launch(hub: &plinth_hub::Hub, app_id: &str) -> Result<LaunchPlan> {
    if hub.is_blocked(app_id)? {
        anyhow::bail!("{app_id} is blocked; it will not run");
    }
    let entry = hub.get(app_id)?.with_context(|| format!("{app_id} is not in the library"))?;
    // The candidate is the pinned-or-latest version: if it declares a
    // capability with no decision yet, consent is asked for it, not
    // silently skipped in favor of an older version.
    let candidate = entry.active_version().with_context(|| format!("{app_id} has no versions"))?.clone();
    let bytes = hub.version_bytes(app_id, &candidate.version)?;
    let pkg = plinth_package::Package::read(&bytes).with_context(|| format!("read the package for {app_id}"))?;
    let declared: Vec<String> = pkg.manifest.capabilities.iter().map(|c| c.name.clone()).collect();
    let pending = hub.needs_consent(app_id, &declared)?;
    if pending.is_empty() {
        return Ok(LaunchPlan::Ready(Box::new(prepare(hub, app_id, pkg)?)));
    }
    // A bad signature is refused outright (`docs/HUB.md` §6.1); an
    // unsigned package shows "unverified publisher".
    let signer = plinth_package::signature::verify(&pkg).context("the package signature does not check out")?;
    let pending = pending
        .iter()
        .map(|c| {
            let why = pkg.manifest.capabilities.iter().find(|m| &m.name == c).map(|m| m.rationale.clone()).unwrap_or_default();
            (c.clone(), why)
        })
        .collect();
    Ok(LaunchPlan::NeedsConsent {
        app_name: pkg.manifest.name.clone(),
        publisher: pkg.manifest.publisher.clone(),
        signer: signer.map(|s| s.key),
        version: candidate.version,
        pending,
    })
}

/// Saves the consent decisions for `version` of `app_id` and prepares the
/// app. `None` (the user cancelled) runs the newest version whose
/// capabilities are all decided, if it is not `version` (`docs/HUB.md`
/// §7.3 step 3); else it returns `Ok(None)` and nothing runs.
pub fn apply_consent(
    hub: &plinth_hub::Hub,
    app_id: &str,
    version: &str,
    decisions: Option<Vec<(String, bool)>>,
) -> Result<Option<PreparedApp>> {
    match decisions {
        Some(decisions) => {
            for (capability, allowed) in decisions {
                let decision = if allowed { plinth_hub::Decision::Allowed } else { plinth_hub::Decision::Refused };
                hub.set_grant(app_id, &capability, decision, version)?;
            }
            let bytes = hub.version_bytes(app_id, version)?;
            let pkg = plinth_package::Package::read(&bytes).with_context(|| format!("read version {version} of {app_id}"))?;
            Ok(Some(prepare(hub, app_id, pkg)?))
        }
        None => {
            let fallback = hub.runnable_version(app_id)?;
            if fallback.version == version {
                eprintln!("[plinth] consent cancelled; {app_id} will not run");
                return Ok(None);
            }
            eprintln!("[plinth] consent cancelled; running the previous version {} of {app_id}", fallback.version);
            let bytes = hub.version_bytes(app_id, &fallback.version)?;
            let pkg = plinth_package::Package::read(&bytes).with_context(|| format!("read version {} of {app_id}", fallback.version))?;
            Ok(Some(prepare(hub, app_id, pkg)?))
        }
    }
}

/// Links a library package and builds its policy from the grants
/// (declared AND allowed is granted; declared and refused is denied).
fn prepare(hub: &plinth_hub::Hub, app_id: &str, pkg: plinth_package::Package) -> Result<PreparedApp> {
    let declared: Vec<String> = pkg.manifest.capabilities.iter().map(|c| c.name.clone()).collect();
    check_hub_trust(&pkg, &declared)?;
    let hub_manage = declared.iter().any(|c| c == plinth_runner_wasmtime::capability::HUB_MANAGE);
    let policy = hub.policy_for(app_id, &declared)?;
    let title = pkg.manifest.name;
    let accent = pkg.manifest.accent.unwrap_or_else(|| "teal".into());
    let component = with_runtime(pkg.component, &declared)?;
    let assets: std::collections::HashMap<String, Vec<u8>> =
        pkg.assets.into_iter().filter_map(|(p, b)| Some((p.strip_prefix("assets/")?.to_owned(), b))).collect();
    let app = HostApp { component, title, accent, app_id: app_id.to_owned(), capabilities: declared, assets };
    Ok(PreparedApp { app, policy, hub_manage })
}

/// What a Hub host process shares across its windows (`docs/HUB.md` §4.2,
/// §12.2): the library, one engine, and the source client for the Hub
/// UI's `search` and `install`.
#[derive(Clone)]
pub struct HubHost {
    pub hub: plinth_hub::Hub,
    pub runner: Arc<Runner>,
    pub sources: Option<Arc<dyn plinth_hub::SourceClient>>,
}

impl HubHost {
    /// The `plinth:hub` backend for a Hub UI window.
    fn backend(&self) -> Box<dyn plinth_runner_wasmtime::hub::HubBackend> {
        Box::new(match &self.sources {
            Some(sources) => plinth_hub::HubService::with_sources(self.hub.clone(), sources.clone()),
            None => plinth_hub::HubService::new(self.hub.clone()),
        })
    }
}

/// How often the host asks a Hub UI window for launch requests.
pub const HUB_LAUNCH_POLL: Duration = Duration::from_millis(100);

/// Opens a prepared library app in a new window. A Hub UI app (one with
/// `hub.manage`) gets a `plinth:hub` backend and a poll loop that opens
/// each app that it launches (`poll_hub_launches`).
pub fn open_prepared(cx: &mut App, host: &HubHost, prepared: PreparedApp) -> gpui::WindowHandle<PlinthRoot> {
    let PreparedApp { app, policy, hub_manage } = prepared;
    let backend = hub_manage.then(|| host.backend());
    let window = open_app_with_hub(cx, host.runner.clone(), app, policy, backend);
    if hub_manage {
        let host = host.clone();
        cx.spawn(async move |cx| loop {
            cx.background_executor().timer(HUB_LAUNCH_POLL).await;
            let open = cx.update(|cx| poll_hub_launches(cx, window, &host));
            if !open {
                return;
            }
        })
        .detach();
    }
    window
}

/// Drains the launch requests of the Hub UI in `window` and launches each
/// app (`launch_from_hub`). Returns `false` when the window is closed, so
/// the caller stops polling. A launch that fails (a blocked app, an app
/// that is not in the library) is logged; the Hub UI shows the state of
/// the library itself.
pub fn poll_hub_launches(cx: &mut App, window: gpui::WindowHandle<PlinthRoot>, host: &HubHost) -> bool {
    let Ok(ids) = window.update(cx, |root, _, _| root.take_hub_launches()) else { return false };
    for id in ids {
        if let Err(e) = launch_from_hub(cx, host, &id) {
            eprintln!("[plinth] cannot open {id}: {e:#}");
        }
    }
    true
}

/// Opens library app `app_id` in a new window of this process
/// (`docs/HUB.md` §4.2): at once if every capability is decided, else
/// after the consent window (`docs/HUB.md` §7.3), which opens in this
/// process too and does not block the other windows.
pub fn launch_from_hub(cx: &mut App, host: &HubHost, app_id: &str) -> Result<()> {
    match plan_launch(&host.hub, app_id)? {
        LaunchPlan::Ready(prepared) => {
            open_prepared(cx, host, *prepared);
        }
        LaunchPlan::NeedsConsent { app_name, publisher, signer, version, pending } => {
            let host = host.clone();
            let app_id = app_id.to_owned();
            consent::open(cx, &app_name, &publisher, signer.as_deref(), &pending, move |decisions, cx| {
                match apply_consent(&host.hub, &app_id, &version, decisions) {
                    Ok(Some(prepared)) => {
                        open_prepared(cx, &host, prepared);
                    }
                    Ok(None) => {}
                    Err(e) => eprintln!("[plinth] cannot open {app_id}: {e:#}"),
                }
            });
        }
    }
    Ok(())
}

/// Runs one library app from the Hub (`docs/HUB.md` §7.3, §9): refuses a
/// blocked app, shows the consent screen for any declared capability that
/// has no grant yet, saves the decisions, then opens the app with a policy
/// built from the grants. Returns `Ok(())` without running anything if the
/// user cancels consent. No source client: the Hub UI's `search` and
/// `install` report that this host cannot read sources.
pub fn run_from_hub(hub: &plinth_hub::Hub, app_id: &str) -> Result<()> {
    run_from_hub_with_sources(hub, app_id, None)
}

/// Like `run_from_hub`, with a source client for the Hub UI's `search` and
/// `install` (`docs/HUB.md` §5). The process keeps running while any
/// window is open: if the app is the Hub UI, each app that it launches
/// opens in a new window of this process (`docs/HUB.md` §4.2).
pub fn run_from_hub_with_sources(hub: &plinth_hub::Hub, app_id: &str, sources: Option<Arc<dyn plinth_hub::SourceClient>>) -> Result<()> {
    let prepared = match plan_launch(hub, app_id)? {
        LaunchPlan::Ready(prepared) => *prepared,
        LaunchPlan::NeedsConsent { app_name, publisher, signer, version, pending } => {
            // No application runs yet: the consent window runs its own loop.
            let decisions = consent::show(&app_name, &publisher, signer.as_deref(), &pending);
            match apply_consent(hub, app_id, &version, decisions)? {
                Some(prepared) => prepared,
                None => return Ok(()),
            }
        }
    };
    let host = HubHost { hub: hub.clone(), runner: Arc::new(Runner::new()?), sources };
    gpui_platform::application().run(move |cx: &mut App| {
        init_app(cx);
        open_prepared(cx, &host, prepared);
        cx.activate(true);
    });
    Ok(())
}

/// Installs the default logger (`RUST_LOG` overrides the level).
pub fn init_logging() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).try_init();
}

#[cfg(test)]
mod hub_trust_tests {
    //! `docs/HUB.md` §4.1: only a package signed by a trusted Hub key may
    //! declare `hub.manage`; everyone else is refused at load, with a
    //! clear error, before it ever runs.
    use super::*;

    fn identity(name: &str) -> plinth_package::publisher::PublisherIdentity {
        plinth_package::publisher::PublisherIdentity { name: name.to_owned(), signing_key: ed25519_dalek::SigningKey::generate(&mut rand_core::OsRng) }
    }

    /// A package declaring `hub.manage`, signed by `signer` if given.
    fn hub_package(signer: Option<&plinth_package::publisher::PublisherIdentity>) -> plinth_package::Package {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/counter");
        let fs = plinth_compiler::driver::DiskFs { root };
        let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[]).expect("compile");
        let artifact = artifact.unwrap_or_else(|| {
            let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
            panic!("counter has errors:\n{}", diags.join("\n"))
        });
        let component = artifact.app;
        let publisher = signer.map(|s| s.name.as_str()).unwrap_or("me");
        let toml = format!(
            "id = \"com.example.hub-mini\"\nname = \"Hub mini\"\nversion = \"0.1.0\"\npublisher = \"{publisher}\"\n\n[[capabilities]]\nname = \"hub.manage\"\nrationale = \"manage the library\"\n"
        );
        let cfg = plinth_package::ProjectConfig::parse(&toml).unwrap();
        let manifest = cfg.manifest("1.0", "plinth-rt/1.0", None, &component);
        let mut pkg = plinth_package::Package { manifest, component, assets: Vec::new(), signature: None };
        if let Some(signer) = signer {
            pkg.signature = Some(plinth_package::signature::sign(&pkg, signer).unwrap());
        }
        pkg
    }

    #[test]
    fn unsigned_package_declaring_hub_manage_is_refused() {
        let pkg = hub_package(None);
        let err = check_hub_trust(&pkg, &["hub.manage".to_owned()]).unwrap_err();
        assert!(format!("{err:#}").contains("unsigned"), "{err:#}");
    }

    /// Both halves of the `PLINTH_HUB_TRUSTED_KEYS` check share one test
    /// (like `split.rs`'s single-env-var tests): two tests racing to set
    /// and unset the same process-wide variable would be flaky.
    #[test]
    fn trusted_keys_env_var_gates_an_untrusted_vs_a_trusted_signer() {
        let signer = identity("Acme Hub");
        let pkg = hub_package(Some(&signer));
        // SAFETY: this test is the only one in this binary that reads the variable.
        unsafe { std::env::remove_var("PLINTH_HUB_TRUSTED_KEYS") };
        let err = check_hub_trust(&pkg, &["hub.manage".to_owned()]).unwrap_err();
        assert!(format!("{err:#}").contains("does not trust"), "{err:#}");

        unsafe { std::env::set_var("PLINTH_HUB_TRUSTED_KEYS", signer.key_id()) };
        let result = check_hub_trust(&pkg, &["hub.manage".to_owned()]);
        unsafe { std::env::remove_var("PLINTH_HUB_TRUSTED_KEYS") };
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn a_package_that_does_not_declare_hub_manage_is_unaffected() {
        let pkg = hub_package(None);
        assert!(check_hub_trust(&pkg, &[]).is_ok());
    }
}
