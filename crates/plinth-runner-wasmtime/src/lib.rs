//! A Wasm runner adapter on wasmtime (SPEC.md §9.2).
//!
//! The runner loads an app component, checks its imports against the
//! `plinth:app` world, and calls `init` and `on-event`. Each call has a time
//! limit (epoch interruption) and the guest has a memory cap.

use anyhow::{Context as _, Result, bail};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use wasmtime::component::{Component, HasSelf, Linker, types::ComponentItem};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};

pub mod files;
pub mod hub;
pub mod kv;
pub mod net;
pub mod policy;
pub mod requests;
pub mod timers;

use hub::HubBackend;
use kv::Kv;
use policy::{DeniedReason, Policy};
use requests::RequestQueue;
use timers::TimerQueue;

mod bindings {
    wasmtime::component::bindgen!({
        path: "../../wit/plinth",
        world: "app",
    });
}

use bindings::App;
use bindings::plinth::app::error::{DeniedReason as WitDeniedReason, HostError};

/// Capability names (SPEC.md §11), from the shared map in `plinth-link`
/// (`docs/HUB.md` §12.3) rather than a duplicated list here.
pub mod capability {
    pub use plinth_link::capabilities::{CLIPBOARD_READ, CLIPBOARD_WRITE, HUB_MANAGE, STORE_KV};
}

impl From<DeniedReason> for WitDeniedReason {
    fn from(r: DeniedReason) -> Self {
        match r {
            DeniedReason::Undeclared => WitDeniedReason::Undeclared,
            DeniedReason::Refused => WitDeniedReason::Refused,
            DeniedReason::Unsupported => WitDeniedReason::Unsupported,
        }
    }
}

/// Runs `policy.check(capability)`, turning a denial into a `HostError`.
fn require(policy: &Policy, capability: &str) -> Result<(), HostError> {
    policy.check(capability).map_err(|r| HostError::Denied(r.into()))
}

/// A host clipboard. `plinth-host-desktop` implements this on gpui-ce's
/// clipboard API; tests use an in-memory stub.
pub trait Clipboard: Send {
    fn write_text(&mut self, text: &str);
    fn read_text(&mut self) -> Option<String>;
}

/// Which `plinth:dialog` call opened a pending dialog request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogKind {
    Alert,
    Confirm,
    Prompt,
}

/// A dialog request the guest opened and the host has not answered yet.
/// `plinth-host-desktop` shows it as a modal; tests answer it directly
/// through `Guest::answer_dialog`.
#[derive(Clone, Debug)]
pub struct PendingDialog {
    pub id: u32,
    pub kind: DialogKind,
    pub message: String,
}

/// An in-memory clipboard, for tests and hosts with no system clipboard.
#[derive(Default)]
pub struct MemoryClipboard(Option<String>);

impl Clipboard for MemoryClipboard {
    fn write_text(&mut self, text: &str) {
        self.0 = Some(text.to_owned());
    }
    fn read_text(&mut self) -> Option<String> {
        self.0.clone()
    }
}

/// The prefix of every import that an app may use (SPEC.md §4.1, level 3).
pub const ALLOWED_IMPORT_PREFIX: &str = "plinth:app/";

/// Limits for one guest.
#[derive(Clone, Debug)]
pub struct Limits {
    /// The maximum linear memory of the guest, in bytes.
    pub memory_bytes: usize,
    /// The maximum time of one `init` or `on-event` call.
    pub call_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self { memory_bytes: 64 << 20, call_timeout: Duration::from_secs(2) }
    }
}

/// How often the epoch ticker increments the engine epoch.
const EPOCH_TICK: Duration = Duration::from_millis(10);

struct HostState {
    commits: Vec<Vec<u8>>,
    /// Messages from `dev.log`, kept for tests and the dev tools.
    logs: Vec<String>,
    /// Uncaught app errors from `error.report` (SPEC.md §5.6). The app
    /// keeps running after each one.
    errors: Vec<String>,
    limits: StoreLimits,
    policy: Policy,
    kv: Kv,
    timers: TimerQueue,
    clipboard: Box<dyn Clipboard>,
    monotonic_origin: Instant,
    requests: RequestQueue,
    /// Dialog requests the guest opened that no `completion` event has
    /// answered yet (SPEC.md §8.4, §8.5).
    dialogs: Vec<PendingDialog>,
    /// `net.fetch` results a worker thread finished, waiting for the host
    /// to deliver them as a `completion` event (SPEC.md §8.4, §8.5). An
    /// `Arc<Mutex<_>>` because the worker thread writes to it directly:
    /// it never touches `HostState`/the wasmtime `Store`, which are not
    /// `Send` across the call.
    net_results: std::sync::Arc<std::sync::Mutex<Vec<(u32, plinth_protocol::Value)>>>,
    /// Overrides `timezone-offset` with a fixed value (minutes east of
    /// UTC), for tests. `None` uses the real OS offset.
    fake_timezone_offset_minutes: Option<i32>,
    /// The `plinth:hub` backend (`docs/HUB.md` §4.1, §12.2), present only
    /// for a guest the host trusted with `hub.manage`.
    hub: Option<Box<dyn HubBackend>>,
    /// The app's private space for `plinth:files` (core 1.11). Unavailable
    /// until the host gives one with `Guest::set_files`.
    files: files::Files,
}

impl bindings::plinth::app::ui::Host for HostState {
    fn commit(&mut self, ops: Vec<u8>) {
        self.commits.push(ops);
    }
}

impl bindings::plinth::app::dev::Host for HostState {
    fn log(&mut self, msg: String) {
        log::info!(target: "plinth::app", "{msg}");
        self.logs.push(msg);
    }
}

impl bindings::plinth::app::error::Host for HostState {
    fn report(&mut self, message: String) {
        log::error!(target: "plinth::app", "{message}");
        self.errors.push(message);
    }
}

impl bindings::plinth::app::time::Host for HostState {
    fn now(&mut self) -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64
    }

    fn monotonic_now(&mut self) -> u64 {
        self.monotonic_origin.elapsed().as_millis() as u64
    }

    fn set_timer(&mut self, ms: u32, repeat: bool) -> Result<u32, HostError> {
        // `time` needs no capability (SPEC.md §11): always allowed.
        Ok(self.timers.set(Instant::now(), ms, repeat))
    }

    fn cancel_timer(&mut self, timer: u32) {
        self.timers.cancel(timer);
    }

    fn timezone_offset(&mut self, ms: i64) -> i32 {
        if let Some(fake) = self.fake_timezone_offset_minutes {
            return fake;
        }
        let secs = ms.div_euclid(1000);
        let dt = time::OffsetDateTime::from_unix_timestamp(secs).unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
        time::UtcOffset::local_offset_at(dt).map(|o| o.whole_minutes() as i32).unwrap_or(0)
    }
}

impl bindings::plinth::app::store::Host for HostState {
    fn kv_get(&mut self, key: String) -> Result<Option<String>, HostError> {
        require(&self.policy, capability::STORE_KV)?;
        Ok(self.kv.get(&key))
    }

    fn kv_set(&mut self, key: String, value: String) -> Result<(), HostError> {
        require(&self.policy, capability::STORE_KV)?;
        self.kv.set(key, value).map_err(|e| {
            log::warn!("kv write failed: {e:#}");
            HostError::Denied(WitDeniedReason::Unsupported)
        })
    }

    fn kv_delete(&mut self, key: String) -> Result<(), HostError> {
        require(&self.policy, capability::STORE_KV)?;
        self.kv.delete(&key).map_err(|e| {
            log::warn!("kv write failed: {e:#}");
            HostError::Denied(WitDeniedReason::Unsupported)
        })
    }

    fn kv_keys(&mut self) -> Result<Vec<String>, HostError> {
        require(&self.policy, capability::STORE_KV)?;
        Ok(self.kv.keys())
    }
}

impl bindings::plinth::app::clipboard::Host for HostState {
    fn write_text(&mut self, text: String) -> Result<(), HostError> {
        require(&self.policy, capability::CLIPBOARD_WRITE)?;
        self.clipboard.write_text(&text);
        Ok(())
    }

    fn read_text(&mut self) -> Result<Option<String>, HostError> {
        require(&self.policy, capability::CLIPBOARD_READ)?;
        Ok(self.clipboard.read_text())
    }
}

impl bindings::plinth::app::hub::Host for HostState {
    fn list_apps(&mut self) -> Result<String, HostError> {
        require(&self.policy, capability::HUB_MANAGE)?;
        match &self.hub {
            Some(hub) => hub.list_apps_json().map_err(|_| HostError::Denied(WitDeniedReason::Unsupported)),
            None => Err(HostError::Denied(WitDeniedReason::Unsupported)),
        }
    }

    fn launch(&mut self, id: String) -> Result<(), HostError> {
        require(&self.policy, capability::HUB_MANAGE)?;
        match &mut self.hub {
            Some(hub) => {
                hub.launch(&id);
                Ok(())
            }
            None => Err(HostError::Denied(WitDeniedReason::Unsupported)),
        }
    }

    fn set_grant(&mut self, id: String, capability: String, allowed: bool) -> Result<(), HostError> {
        require(&self.policy, self::capability::HUB_MANAGE)?;
        match &mut self.hub {
            Some(hub) => hub.set_grant(&id, &capability, allowed).map_err(|_| HostError::Denied(WitDeniedReason::Unsupported)),
            None => Err(HostError::Denied(WitDeniedReason::Unsupported)),
        }
    }

    fn block(&mut self, id: String) -> Result<(), HostError> {
        require(&self.policy, capability::HUB_MANAGE)?;
        match &mut self.hub {
            Some(hub) => hub.block(&id).map_err(|_| HostError::Denied(WitDeniedReason::Unsupported)),
            None => Err(HostError::Denied(WitDeniedReason::Unsupported)),
        }
    }

    fn unblock(&mut self, id: String) -> Result<(), HostError> {
        require(&self.policy, capability::HUB_MANAGE)?;
        match &mut self.hub {
            Some(hub) => hub.unblock(&id).map_err(|_| HostError::Denied(WitDeniedReason::Unsupported)),
            None => Err(HostError::Denied(WitDeniedReason::Unsupported)),
        }
    }

    fn list_groups(&mut self) -> Result<String, HostError> {
        require(&self.policy, capability::HUB_MANAGE)?;
        match &self.hub {
            Some(hub) => hub.list_groups_json().map_err(|_| HostError::Denied(WitDeniedReason::Unsupported)),
            None => Err(HostError::Denied(WitDeniedReason::Unsupported)),
        }
    }

    fn create_group(&mut self, name: String) -> Result<(), HostError> {
        require(&self.policy, capability::HUB_MANAGE)?;
        match &mut self.hub {
            Some(hub) => hub.create_group(&name).map_err(|_| HostError::Denied(WitDeniedReason::Unsupported)),
            None => Err(HostError::Denied(WitDeniedReason::Unsupported)),
        }
    }

    fn set_group(&mut self, id: String, group: String, member: bool) -> Result<(), HostError> {
        require(&self.policy, capability::HUB_MANAGE)?;
        match &mut self.hub {
            Some(hub) => hub.set_group(&id, &group, member).map_err(|_| HostError::Denied(WitDeniedReason::Unsupported)),
            None => Err(HostError::Denied(WitDeniedReason::Unsupported)),
        }
    }

    fn remove(&mut self, id: String) -> Result<(), HostError> {
        require(&self.policy, capability::HUB_MANAGE)?;
        match &mut self.hub {
            Some(hub) => hub.remove(&id).map_err(|_| HostError::Denied(WitDeniedReason::Unsupported)),
            None => Err(HostError::Denied(WitDeniedReason::Unsupported)),
        }
    }

    fn search(&mut self, query: String) -> u32 {
        let id = self.requests.open();
        let job = self.hub_job(|hub| hub.search(&query));
        match job {
            // Denied: the completion result is null (`wit/plinth/app.wit`).
            Err(_) => self.net_results.lock().unwrap().push((id, plinth_protocol::Value::Null)),
            Ok(job) => {
                let tx = self.net_results.clone();
                std::thread::spawn(move || {
                    let result = match job() {
                        Ok(json) => plinth_protocol::Value::Str(json),
                        Err(e) => {
                            log::warn!("hub search failed: {e}");
                            plinth_protocol::Value::Null
                        }
                    };
                    tx.lock().unwrap().push((id, result));
                });
            }
        }
        id
    }

    fn install(&mut self, app: String) -> u32 {
        let id = self.requests.open();
        match self.hub_job(|hub| hub.install(&app)) {
            // Denied: the completion result is the reason text.
            Err(reason) => self.net_results.lock().unwrap().push((id, plinth_protocol::Value::Str(denied_text(reason).to_owned()))),
            Ok(job) => {
                let tx = self.net_results.clone();
                std::thread::spawn(move || {
                    let result = match job() {
                        Ok(_) => plinth_protocol::Value::Null,
                        Err(e) => plinth_protocol::Value::Str(e),
                    };
                    tx.lock().unwrap().push((id, result));
                });
            }
        }
        id
    }

    fn app_info(&mut self, id: String) -> Result<String, HostError> {
        require(&self.policy, capability::HUB_MANAGE)?;
        match &self.hub {
            Some(hub) => hub.app_info_json(&id).map_err(|_| HostError::Denied(WitDeniedReason::Unsupported)),
            None => Err(HostError::Denied(WitDeniedReason::Unsupported)),
        }
    }

    fn pin(&mut self, id: String, version: String) -> Result<(), HostError> {
        require(&self.policy, capability::HUB_MANAGE)?;
        match &mut self.hub {
            Some(hub) => hub.pin(&id, &version).map_err(|_| HostError::Denied(WitDeniedReason::Unsupported)),
            None => Err(HostError::Denied(WitDeniedReason::Unsupported)),
        }
    }

    fn block_publisher(&mut self, key: String) -> Result<(), HostError> {
        require(&self.policy, capability::HUB_MANAGE)?;
        match &mut self.hub {
            Some(hub) => hub.block_publisher(&key).map_err(|_| HostError::Denied(WitDeniedReason::Unsupported)),
            None => Err(HostError::Denied(WitDeniedReason::Unsupported)),
        }
    }

    fn unblock_publisher(&mut self, key: String) -> Result<(), HostError> {
        require(&self.policy, capability::HUB_MANAGE)?;
        match &mut self.hub {
            Some(hub) => hub.unblock_publisher(&key).map_err(|_| HostError::Denied(WitDeniedReason::Unsupported)),
            None => Err(HostError::Denied(WitDeniedReason::Unsupported)),
        }
    }

    fn check_updates(&mut self, app: String) -> u32 {
        let id = self.requests.open();
        match self.hub_job(|hub| hub.check_updates(&app)) {
            // Denied: the completion result is null, as for `search`.
            Err(_) => self.net_results.lock().unwrap().push((id, plinth_protocol::Value::Null)),
            Ok(job) => {
                let tx = self.net_results.clone();
                std::thread::spawn(move || {
                    let result = match job() {
                        Ok(json) => plinth_protocol::Value::Str(json),
                        // A failed check still answers with JSON, so the
                        // app can tell "denied" (null) from "failed".
                        Err(e) => plinth_protocol::Value::Str(serde_json::json!({ "updates": [], "errors": [e] }).to_string()),
                    };
                    tx.lock().unwrap().push((id, result));
                });
            }
        }
        id
    }

    fn update(&mut self, app: String) -> u32 {
        let id = self.requests.open();
        match self.hub_job(|hub| hub.update(&app)) {
            Err(reason) => self.net_results.lock().unwrap().push((id, plinth_protocol::Value::Str(denied_text(reason).to_owned()))),
            Ok(job) => {
                let tx = self.net_results.clone();
                std::thread::spawn(move || {
                    let result = match job() {
                        Ok(_) => plinth_protocol::Value::Null,
                        Err(e) => plinth_protocol::Value::Str(e),
                    };
                    tx.lock().unwrap().push((id, result));
                });
            }
        }
        id
    }
}

impl HostState {
    /// Builds a `plinth:hub` worker job, or the reason the call is denied.
    fn hub_job(&self, make: impl FnOnce(&dyn HubBackend) -> hub::HubJob) -> Result<hub::HubJob, DeniedReason> {
        self.policy.check(capability::HUB_MANAGE)?;
        match &self.hub {
            Some(hub) => Ok(make(hub.as_ref())),
            None => Err(DeniedReason::Unsupported),
        }
    }
}

/// The `"denied:<reason>"` text that `lastError()` and async results use.
fn denied_text(reason: DeniedReason) -> &'static str {
    match reason {
        DeniedReason::Undeclared => "denied:undeclared",
        DeniedReason::Refused => "denied:refused",
        DeniedReason::Unsupported => "denied:unsupported",
    }
}

impl bindings::plinth::app::dialog::Host for HostState {
    fn alert(&mut self, message: String) -> u32 {
        let id = self.requests.open();
        self.dialogs.push(PendingDialog { id, kind: DialogKind::Alert, message });
        id
    }

    fn confirm(&mut self, message: String) -> u32 {
        let id = self.requests.open();
        self.dialogs.push(PendingDialog { id, kind: DialogKind::Confirm, message });
        id
    }

    fn prompt(&mut self, message: String) -> u32 {
        let id = self.requests.open();
        self.dialogs.push(PendingDialog { id, kind: DialogKind::Prompt, message });
        id
    }
}

impl bindings::plinth::app::net::Host for HostState {
    fn fetch(&mut self, url: String, method: String, headers: Vec<(String, String)>, body: Option<String>) -> u32 {
        let id = self.requests.open();
        match net::check_url(&self.policy, &url) {
            Err(reason) => self.net_results.lock().unwrap().push((id, net::denied(reason))),
            Ok(()) => {
                let tx = self.net_results.clone();
                std::thread::spawn(move || {
                    let outcome = net::blocking_fetch(&url, &method, &headers, body.as_deref());
                    tx.lock().unwrap().push((id, outcome));
                });
            }
        }
        id
    }
}

impl HostState {
    fn files_call(&mut self, op: files::Op) -> u32 {
        let id = self.requests.open();
        self.files.submit(&self.policy, id, op, &self.net_results);
        id
    }
}

impl bindings::plinth::app::files::Host for HostState {
    fn read(&mut self, path: String) -> u32 {
        self.files_call(files::Op::Read(path))
    }

    fn write(&mut self, path: String, text: String) -> u32 {
        self.files_call(files::Op::Write(path, text))
    }

    fn list_dir(&mut self, dir: String) -> u32 {
        self.files_call(files::Op::List(dir))
    }

    fn stat(&mut self, path: String) -> u32 {
        self.files_call(files::Op::Stat(path))
    }

    fn remove(&mut self, path: String) -> u32 {
        self.files_call(files::Op::Remove(path))
    }
}

/// The engine is shared by all guests. It owns the epoch ticker thread.
pub struct Runner {
    engine: Engine,
    stop_ticker: Arc<AtomicBool>,
}

impl Runner {
    pub fn new() -> Result<Self> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.epoch_interruption(true);
        let engine = Engine::new(&config)?;

        let stop_ticker = Arc::new(AtomicBool::new(false));
        let (ticker_engine, stop) = (engine.weak(), stop_ticker.clone());
        std::thread::Builder::new().name("plinth-epoch".into()).spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(EPOCH_TICK);
                match ticker_engine.upgrade() {
                    Some(engine) => engine.increment_epoch(),
                    None => break,
                }
            }
        })?;
        Ok(Self { engine, stop_ticker })
    }

    /// Compiles a component and checks its imports. Grants every capability
    /// and uses an in-memory store and clipboard (no policy, for hosts and
    /// tests that do not need host API access).
    pub fn load(&self, bytes: &[u8], limits: Limits) -> Result<Guest> {
        self.load_with_policy(bytes, limits, Policy::allow_all(), Kv::in_memory(), Box::new(MemoryClipboard::default()))
    }

    /// Compiles a component and checks its imports, with a capability
    /// policy and host API implementations (SPEC.md §9.4, §11). No
    /// `plinth:hub` backend (`docs/HUB.md` §4.1): a `hub.manage` call is
    /// `unsupported` even if the policy would allow it.
    pub fn load_with_policy(
        &self,
        bytes: &[u8],
        limits: Limits,
        policy: Policy,
        kv: Kv,
        clipboard: Box<dyn Clipboard>,
    ) -> Result<Guest> {
        self.load_with_policy_and_hub(bytes, limits, policy, kv, clipboard, None)
    }

    /// Like `load_with_policy`, with a `plinth:hub` backend (`docs/HUB.md`
    /// §4.1, §12.2) for a guest the host trusted with `hub.manage`
    /// (`plinth-host-desktop` checks the trusted-signer rule before it
    /// ever reaches here).
    pub fn load_with_policy_and_hub(
        &self,
        bytes: &[u8],
        limits: Limits,
        policy: Policy,
        kv: Kv,
        clipboard: Box<dyn Clipboard>,
        hub: Option<Box<dyn HubBackend>>,
    ) -> Result<Guest> {
        let component = Component::new(&self.engine, bytes)
            .map_err(anyhow::Error::from)
            .context("the artifact is not a valid component")?;
        check_imports(&self.engine, &component)?;

        let mut linker = Linker::<HostState>::new(&self.engine);
        App::add_to_linker::<_, HasSelf<_>>(&mut linker, |s| s)?;

        let state = HostState {
            commits: Vec::new(),
            logs: Vec::new(),
            errors: Vec::new(),
            limits: StoreLimitsBuilder::new().memory_size(limits.memory_bytes).instances(4).build(),
            policy,
            kv,
            timers: TimerQueue::new(),
            clipboard,
            monotonic_origin: Instant::now(),
            requests: RequestQueue::new(),
            dialogs: Vec::new(),
            net_results: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            fake_timezone_offset_minutes: None,
            hub,
            files: files::Files::unavailable(),
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|s| &mut s.limits);
        store.epoch_deadline_trap();
        let ticks = (limits.call_timeout.as_millis() / EPOCH_TICK.as_millis()).max(1) as u64;
        store.set_epoch_deadline(ticks);
        let app = App::instantiate(&mut store, &component, &linker)
            .map_err(anyhow::Error::from)
            .context("instantiate the app")?;
        Ok(Guest { store, app, deadline_ticks: ticks, poisoned: false })
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        self.stop_ticker.store(true, Ordering::Relaxed);
    }
}

/// Rejects any import outside the `plinth:app` world, `wasi:*` included.
fn check_imports(engine: &Engine, component: &Component) -> Result<()> {
    let ty = component.component_type();
    for (name, item) in ty.imports(engine) {
        if !name.starts_with(ALLOWED_IMPORT_PREFIX) {
            bail!("the artifact imports `{name}`, which is not in the plinth:app world");
        }
        if !matches!(item.ty, ComponentItem::ComponentInstance(_)) {
            bail!("the artifact import `{name}` is not an interface");
        }
    }
    Ok(())
}

/// One running app instance.
pub struct Guest {
    store: Store<HostState>,
    app: App,
    deadline_ticks: u64,
    /// Set after a trap. A trapped instance is never called again.
    poisoned: bool,
}

impl Guest {
    /// Returns the `dev.log` messages since the last call.
    pub fn take_logs(&mut self) -> Vec<String> {
        std::mem::take(&mut self.store.data_mut().logs)
    }

    /// Returns the uncaught errors that the guest reported (`error.report`,
    /// SPEC.md §5.6) since the last call. The guest is still running.
    pub fn take_errors(&mut self) -> Vec<String> {
        std::mem::take(&mut self.store.data_mut().errors)
    }

    /// Calls `init` and returns the op buffers that the guest committed.
    pub fn init(&mut self, args: &[u8]) -> Result<Vec<Vec<u8>>> {
        self.call(|app, store| app.call_init(store, args))
    }

    /// Calls `on-event` and returns the op buffers that the guest committed.
    pub fn on_event(&mut self, events: &[u8]) -> Result<Vec<Vec<u8>>> {
        self.call(|app, store| app.call_on_event(store, events))
    }

    /// Overrides `timezone-offset` with a fixed value (minutes east of
    /// UTC) for tests, instead of the real OS offset.
    pub fn set_fake_timezone_offset(&mut self, minutes: i32) {
        self.store.data_mut().fake_timezone_offset_minutes = Some(minutes);
    }

    /// Gives the guest its private space for `plinth:files` (core 1.11,
    /// `docs/STORAGE.md` §2, §3). Without it, every call is
    /// `denied:unsupported`. Call it before `init`.
    pub fn set_files(&mut self, files: files::Files) {
        self.store.data_mut().files = files;
    }

    /// The soonest time a timer is due, if any is pending. The desktop
    /// host sleeps until this instant (via `gpui`'s executor timers,
    /// SPEC.md §9.4), then calls `fire_due_timers`.
    pub fn next_timer_deadline(&self) -> Option<Instant> {
        self.store.data().timers.next_deadline()
    }

    /// Builds one `on-event` buffer with a `timer` event (protocol
    /// `Event::Timer`, SPEC.md §8.4) for every timer due at or before
    /// `now`, dispatches it, and returns the committed op buffers. Returns
    /// `Ok(vec![])` with no call if no timer is due.
    pub fn fire_due_timers(&mut self, now: Instant) -> Result<Vec<Vec<u8>>> {
        let due = self.store.data_mut().timers.due(now);
        if due.is_empty() {
            return Ok(Vec::new());
        }
        let mut w = plinth_protocol::Writer::new();
        for timer in due {
            w.event(&plinth_protocol::Event::Timer { timer });
        }
        self.on_event(w.as_bytes())
    }

    /// The dialog requests the guest opened that are still waiting for an
    /// answer (SPEC.md §8.4, §8.5). The desktop host polls this the same
    /// way it polls timers; a test reads it to simulate the host answering.
    pub fn pending_dialogs(&self) -> &[PendingDialog] {
        &self.store.data().dialogs
    }

    /// Drains the `net.fetch` requests a worker thread has finished since
    /// the last call (SPEC.md §8.4, §8.5). The desktop host polls this the
    /// same way it polls timers and dialogs, and answers each one with
    /// `answer_dialog` (the same generic completion delivery).
    pub fn poll_net_results(&mut self) -> Vec<(u32, plinth_protocol::Value)> {
        std::mem::take(&mut *self.store.data().net_results.lock().unwrap())
    }

    /// Drains the `plinth:hub` `launch` requests the guest made since the
    /// last call (`docs/HUB.md` §4.1, §4.2). Empty when the guest has no
    /// `plinth:hub` backend. The host (`plinth-host-desktop`) polls this
    /// the same way it polls timers and dialogs, and opens a window with
    /// `open_app` for each id.
    pub fn take_hub_launches(&mut self) -> Vec<String> {
        match self.store.data_mut().hub.as_mut() {
            Some(hub) => hub.take_launches(),
            None => Vec::new(),
        }
    }

    /// Answers a request (for example a dialog) by delivering a
    /// `completion` event (protocol `Event::Completion`, SPEC.md §8.4)
    /// with `result`, and returns the committed op buffers. Answering an
    /// id that is not open (already answered, or unknown) is not an error:
    /// the guest ignores it (SPEC.md §8.4).
    pub fn answer_dialog(&mut self, id: u32, result: plinth_protocol::Value) -> Result<Vec<Vec<u8>>> {
        self.store.data_mut().requests.close(id);
        self.store.data_mut().dialogs.retain(|d| d.id != id);
        let mut w = plinth_protocol::Writer::new();
        w.event(&plinth_protocol::Event::Completion { request: id, result });
        self.on_event(w.as_bytes())
    }

    fn call(&mut self, f: impl FnOnce(&App, &mut Store<HostState>) -> wasmtime::Result<()>) -> Result<Vec<Vec<u8>>> {
        if self.poisoned {
            bail!("the app stopped earlier");
        }
        self.store.set_epoch_deadline(self.deadline_ticks);
        let result = f(&self.app, &mut self.store);
        let commits = std::mem::take(&mut self.store.data_mut().commits);
        if let Err(e) = result {
            self.poisoned = true;
            let interrupted = matches!(e.downcast_ref::<wasmtime::Trap>(), Some(wasmtime::Trap::Interrupt));
            let e = anyhow::Error::from(e);
            if interrupted {
                return Err(e.context("the app is not responding (time limit reached)"));
            }
            return Err(e);
        }
        Ok(commits)
    }
}
