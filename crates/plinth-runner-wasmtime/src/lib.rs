//! A Wasm runner adapter (SPEC.md §9.2): the host state and the host APIs of
//! the `plinth:app` world, and two engines that run apps with them.
//!
//! The runner loads an app, and calls `init` and `on-event`. Each call has a
//! time limit and the guest has a memory cap. The engines:
//!
//! - `wasmtime` (feature `wasmtime`, on by default): the desktop and Android
//!   engine. It runs the component, with an epoch time limit.
//! - `wasmi` (feature `wasmi`): an interpreter, for iOS (no JIT there) and
//!   for tests. It runs the linked core module, with a fuel time limit.
//!
//! `HostState` and all host APIs are the same for both engines. With
//! `--no-default-features --features wasmi`, wasmtime is not in the
//! dependency tree (`docs/PRODUCTION.md` R5).

#[cfg(not(any(feature = "wasmtime", feature = "wasmi")))]
compile_error!("plinth-runner-wasmtime needs at least one engine: enable feature `wasmtime` or `wasmi`");

use anyhow::{Result, bail};
use std::time::{Duration, Instant};

pub mod app_info;
pub mod files;
pub mod hub;
pub mod kv;
pub mod locale;
pub mod locale_format;
pub mod net;
pub mod notify;
pub mod policy;
pub mod requests;
pub mod secrets;
pub mod sql;
pub mod timers;
#[cfg(feature = "wasmi")]
pub mod wasmi_core;
#[cfg(feature = "wasmi")]
mod wasmi_engine;
#[cfg(feature = "wasmtime")]
mod wasmtime_engine;

#[cfg(feature = "wasmtime")]
pub use wasmtime_engine::WasmtimeGuest;

use hub::HubBackend;
use kv::Kv;
use notify::NotifyBackend;
use policy::{DeniedReason, Policy};
use requests::RequestQueue;
use secrets::{SecretsBackend, SecretsScope};
use timers::TimerQueue;

#[cfg(feature = "wasmtime")]
mod bindings {
    wasmtime::component::bindgen!({
        path: "../../wit/plinth",
        world: "app",
    });
}

// Without wasmtime, the same traits and types, written by hand.
#[cfg(not(feature = "wasmtime"))]
#[path = "host_traits.rs"]
mod bindings;

use bindings::plinth::app::error::{DeniedReason as WitDeniedReason, HostError};

/// Capability names (SPEC.md §11), from the shared map in `plinth-link`
/// (`docs/HUB.md` §12.3) rather than a duplicated list here.
pub mod capability {
    pub use plinth_link::capabilities::{CLIPBOARD_READ, CLIPBOARD_WRITE, HUB_MANAGE, NOTIFY, STORE_KV};
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

/// The guest's memory cap, plus the size of its memory so far (it only
/// grows: the allocator never gives memory back), for `Guest::memory_bytes`.
/// Plain numbers, not a wrapper over one engine's own `StoreLimits`: the
/// wasmtime and wasmi `ResourceLimiter` traits take different error types,
/// so `Limiter` gets two trait impls (SPEC.md §9.2 step 3; one in
/// `wasmtime_engine.rs`, one below), both reading and updating these same
/// fields.
struct Limiter {
    memory_cap: usize,
    memory: usize,
}

/// Whether growing to `desired` is allowed under `cap` and the module's own
/// declared `maximum`, shared by both `ResourceLimiter` impls.
fn allow_growth(cap: usize, desired: usize, maximum: Option<usize>) -> bool {
    desired <= cap && maximum.is_none_or(|max| desired <= max)
}

/// `wasmi::ResourceLimiter` is a re-export of `wasmi_core::ResourceLimiter`
/// (the same trait), so this one impl covers the wasmi backend too.
#[cfg(feature = "wasmi")]
impl ::wasmi_core::ResourceLimiter for Limiter {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> Result<bool, ::wasmi_core::LimiterError> {
        let ok = allow_growth(self.memory_cap, desired, maximum);
        if ok {
            self.memory = self.memory.max(desired);
        }
        Ok(ok)
    }
    fn memory_grow_failed(&mut self, _error: &::wasmi_core::MemoryError) -> Result<(), ::wasmi_core::LimiterError> {
        Ok(())
    }
    fn table_growing(
        &mut self,
        _current: usize,
        _desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool, ::wasmi_core::LimiterError> {
        Ok(true)
    }
    fn table_grow_failed(&mut self, _error: &::wasmi_core::TableError) -> Result<(), ::wasmi_core::LimiterError> {
        Ok(())
    }
    fn instances(&self) -> usize {
        4
    }
    fn tables(&self) -> usize {
        10_000
    }
    fn memories(&self) -> usize {
        10_000
    }
}

struct HostState {
    commits: Vec<Vec<u8>>,
    /// Messages from `dev.log`, kept for tests and the dev tools.
    logs: Vec<String>,
    /// Uncaught app errors from `error.report` (SPEC.md §5.6). The app
    /// keeps running after each one.
    errors: Vec<String>,
    limits: Limiter,
    policy: Policy,
    kv: Kv,
    timers: TimerQueue,
    /// When the host last fired the frame timers (core 1.12); `None` before
    /// the first frame and after the last frame timer stops.
    last_frame: Option<Instant>,
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
    /// The `plinth:notify` backend (core 1.12). A `MemoryNotify` until the
    /// host gives its own with `Guest::set_notify`.
    notify: Box<dyn NotifyBackend>,
    /// The `plinth:secrets` backend (core 1.13). An `UnsupportedSecrets`
    /// until the host gives its own with `Guest::set_secrets`.
    secrets: Box<dyn SecretsBackend>,
    /// Whose secrets the guest reads (the app id and the instance).
    secrets_scope: SecretsScope,
    /// The `store.sql` database (`docs/SQL.md`), made at its first use in
    /// the folder of the private space (`files`).
    sql: Option<sql::SqlStore>,
    /// The locale that the guest reads (`plinth:locale`, core 1.13); `None`
    /// until the first read, which takes the OS locale.
    locale: Option<String>,
    /// True when the host or a test fixed the locale (`Guest::set_locale`):
    /// `Guest::refresh_locale` then does not read the OS again.
    locale_fixed: bool,
    /// A test clock (`Guest::use_test_clock`, `plinth test`): the clocks
    /// and the timers read it instead of the real time. `(now, the epoch
    /// time of now in ms)`.
    test_clock: Option<(Instant, u64)>,
    /// The app's manifest version (`plinth:app-info`, core 1.13,
    /// `PRODUCTION.md` D4); `None` until the host sets one with
    /// `Guest::set_app_version`. Without it, `mark-data-version` and
    /// `Guest::finish_start` do nothing.
    app_version: Option<String>,
    /// The stored data version (`plinth:app-info`): the last app version
    /// whose `onUpgrade` step finished (or, with no hook, the last version
    /// that started). Loaded from `kv.folder()`'s `data-version` file at
    /// the first read; kept here only when `kv` has no folder (tests,
    /// `plinth test`).
    data_version: Option<String>,
    /// True once `data_version` was loaded from disk (or confirmed there
    /// is no file), so a later read does not re-read it.
    data_version_loaded: bool,
    /// True once `data-version()` returned a version to the guest (an
    /// upgrade was pending): `Guest::finish_start` then leaves storing the
    /// version to the guest's `mark-data-version()` call instead of
    /// storing it itself.
    upgrade_claimed: bool,
}

impl HostState {
    /// The state of a new guest. Both engines make it the same way.
    fn new(
        limits: &Limits,
        policy: Policy,
        kv: Kv,
        clipboard: Box<dyn Clipboard>,
        hub: Option<Box<dyn HubBackend>>,
    ) -> Self {
        HostState {
            commits: Vec::new(),
            logs: Vec::new(),
            errors: Vec::new(),
            limits: Limiter { memory_cap: limits.memory_bytes, memory: 0 },
            policy,
            kv,
            timers: TimerQueue::new(),
            last_frame: None,
            clipboard,
            monotonic_origin: Instant::now(),
            requests: RequestQueue::new(),
            dialogs: Vec::new(),
            net_results: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            fake_timezone_offset_minutes: None,
            hub,
            files: files::Files::unavailable(),
            notify: Box::new(notify::MemoryNotify::default()),
            secrets: Box::new(secrets::UnsupportedSecrets),
            secrets_scope: SecretsScope::default(),
            sql: None,
            locale: None,
            locale_fixed: false,
            test_clock: None,
            app_version: None,
            data_version: None,
            data_version_loaded: false,
            upgrade_claimed: false,
        }
    }

    /// The time of the timers: the test clock, else the real time.
    fn instant_now(&self) -> Instant {
        self.test_clock.map_or_else(Instant::now, |(now, _)| now)
    }

    /// The locale of the app: the fixed one, else the OS locale at the first
    /// read (`docs/I18N.md` §3.1).
    fn current_locale(&mut self) -> &str {
        self.locale.get_or_insert_with(locale::os_locale)
    }

    /// The path of the `data-version` file, next to `kv.json`
    /// (`crate::kv::Kv::open`); `None` when `kv` has no folder.
    fn data_version_path(&self) -> Option<std::path::PathBuf> {
        self.kv.folder().map(|f| f.join("data-version"))
    }

    /// The stored data version (`plinth:app-info`), loading it from disk
    /// on the first call.
    fn current_data_version(&mut self) -> Option<String> {
        if !self.data_version_loaded {
            self.data_version_loaded = true;
            if let Some(path) = self.data_version_path() {
                self.data_version = app_info::read(&path);
            }
        }
        self.data_version.clone()
    }

    /// Stores `version` as the data version: in memory, and in the
    /// `data-version` file when `kv` has a folder.
    fn store_data_version(&mut self, version: &str) {
        self.data_version = Some(version.to_owned());
        self.data_version_loaded = true;
        if let Some(path) = self.data_version_path() {
            let _ = app_info::write(&path, version);
        }
    }
}

/// The local time zone offset of the OS at `secs` (Unix time), in minutes
/// east of UTC. 0 if the OS does not know it.
#[cfg(not(target_os = "ios"))]
fn local_offset_minutes(secs: i64) -> i32 {
    let dt = time::OffsetDateTime::from_unix_timestamp(secs).unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    time::UtcOffset::local_offset_at(dt).map(|o| o.whole_minutes() as i32).unwrap_or(0)
}

/// As above, for iOS: the `time` crate does not build there (see
/// `Cargo.toml`), so this calls `localtime_r` and reads `tm_gmtoff`.
#[cfg(target_os = "ios")]
fn local_offset_minutes(secs: i64) -> i32 {
    let t = secs as libc::time_t;
    // SAFETY: `tm` is plain data, so all-zero is valid; `localtime_r` writes
    // only into `tm` and returns null on failure.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() { 0 } else { (tm.tm_gmtoff / 60) as i32 }
    }
}

/// The event buffer that tells a guest that its locale changed: a
/// `lifecycle` event with kind `LOCALE` (core 1.13, `docs/I18N.md` §3.1). An
/// older core ignores it.
pub fn locale_event() -> Vec<u8> {
    let mut w = plinth_protocol::Writer::new();
    w.event(&plinth_protocol::Event::Lifecycle { kind: plinth_protocol::lifecycle_kind::LOCALE });
    w.as_bytes().to_vec()
}

/// 64 bits of entropy for `Math.random`: the standard library's random
/// hash keys (from the OS) mixed with the clock. Not for security.
fn fresh_seed() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
    h.finish()
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
        if let Some((_, epoch_ms)) = self.test_clock {
            return epoch_ms;
        }
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64
    }

    fn monotonic_now(&mut self) -> u64 {
        self.instant_now().saturating_duration_since(self.monotonic_origin).as_millis() as u64
    }

    fn set_timer(&mut self, ms: u32, repeat: bool) -> Result<u32, HostError> {
        // `time` needs no capability (SPEC.md §11): always allowed.
        let now = self.instant_now();
        Ok(self.timers.set(now, ms, repeat))
    }

    fn cancel_timer(&mut self, timer: u32) {
        self.timers.cancel(timer);
    }

    fn set_frame_timer(&mut self) -> Result<u32, HostError> {
        // `time` needs no capability (SPEC.md §11): always allowed.
        let now = self.instant_now();
        Ok(self.timers.set_frame(now))
    }

    fn timezone_offset(&mut self, ms: i64) -> i32 {
        if let Some(fake) = self.fake_timezone_offset_minutes {
            return fake;
        }
        local_offset_minutes(ms.div_euclid(1000))
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
            // A set over the quota answers `denied:refused`, as the web host
            // does (`docs/STORAGE.md` S13); `kv.lastError()` shows it.
            if e.is::<kv::QuotaExceeded>() {
                return HostError::Denied(WitDeniedReason::Refused);
            }
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

impl bindings::plinth::app::notify::Host for HostState {
    fn show(&mut self, title: String, body: Option<String>, tag: Option<String>) -> u32 {
        let id = self.requests.open();
        let result = notify::show(&self.policy, self.notify.as_mut(), &title, body.as_deref(), tag.as_deref());
        self.net_results.lock().unwrap().push((id, result));
        id
    }

    fn cancel(&mut self, tag: String) -> Result<(), HostError> {
        notify::cancel(&self.policy, self.notify.as_mut(), &tag).map_err(|r| HostError::Denied(r.into()))
    }
}

impl bindings::plinth::app::secrets::Host for HostState {
    fn set(&mut self, name: String, value: String) -> u32 {
        let id = self.requests.open();
        let result = secrets::set(&self.policy, self.secrets.as_mut(), &self.secrets_scope, &name, &value);
        self.net_results.lock().unwrap().push((id, result));
        id
    }

    fn get(&mut self, name: String) -> u32 {
        let id = self.requests.open();
        let result = secrets::get(&self.policy, self.secrets.as_mut(), &self.secrets_scope, &name);
        self.net_results.lock().unwrap().push((id, result));
        id
    }

    fn remove(&mut self, name: String) -> u32 {
        let id = self.requests.open();
        let result = secrets::remove(&self.policy, self.secrets.as_mut(), &self.secrets_scope, &name);
        self.net_results.lock().unwrap().push((id, result));
        id
    }

    fn names(&mut self) -> u32 {
        let id = self.requests.open();
        let result = secrets::names(&self.policy, self.secrets.as_mut(), &self.secrets_scope);
        self.net_results.lock().unwrap().push((id, result));
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
                let allow_private = self.policy.allows_local_network();
                std::thread::spawn(move || {
                    let outcome = net::blocking_fetch(&url, &method, &headers, body.as_deref(), allow_private);
                    tx.lock().unwrap().push((id, outcome));
                });
            }
        }
        id
    }
}

impl HostState {
    /// The `store.sql` database, made at the first call. `None` if the
    /// guest has no private space.
    fn sql_store(&mut self) -> Option<&sql::SqlStore> {
        if self.sql.is_none() {
            let dir = sql::database_dir(self.files.root()?);
            // The database follows the quota of the private space (the host
            // sets one per-app quota, `docs/STORAGE.md` S13).
            let limits = sql::Limits { quota: self.files.quota(), ..sql::Limits::default() };
            self.sql = Some(sql::SqlStore::open(dir, limits));
        }
        self.sql.as_ref()
    }

    fn files_call(&mut self, op: files::Op) -> u32 {
        let id = self.requests.open();
        self.files.submit(&self.policy, id, op, &self.net_results);
        id
    }
}

/// `store.sql` (core 1.13, `docs/SQL.md`). The capability check comes
/// first; a denied call, an unknown `kind` and a guest with no private
/// space complete at once with an error, and never trap.
// `plinth:locale` (core 1.13): no capability, as for the time zone.
impl bindings::plinth::app::locale::Host for HostState {
    fn tag(&mut self) -> String {
        self.current_locale().to_owned()
    }

    fn is_rtl(&mut self) -> bool {
        locale::is_rtl(self.current_locale())
    }

    // Formatting (docs/I18N.md §3.7): ICU4X, in `locale_format`.
    fn format_number(&mut self, value: f64, options: String) -> String {
        locale_format::format_number(self.current_locale(), value, &options)
    }

    fn format_date(&mut self, ms: f64, options: String) -> String {
        let offset = if ms.is_finite() { bindings::plinth::app::time::Host::timezone_offset(self, ms as i64) } else { 0 };
        locale_format::format_date(self.current_locale(), ms, offset, &options)
    }

    fn plural_category(&mut self, value: f64, kind: String) -> String {
        locale_format::plural_category(self.current_locale(), value, &kind).to_owned()
    }

    fn format_list(&mut self, items: String, kind: String) -> String {
        locale_format::format_list(self.current_locale(), &items, &kind)
    }

    fn compare(&mut self, a: String, b: String) -> i32 {
        locale_format::compare(self.current_locale(), &a, &b)
    }
}

// `plinth:app-info` (core 1.13, `PRODUCTION.md` D4): no capability, as for
// `plinth:locale`.
impl bindings::plinth::app::app_info::Host for HostState {
    fn data_version(&mut self) -> Option<String> {
        // Step B: the guest only ever sees a version when an upgrade is
        // pending (a stored version exists and is lower than the current
        // app version). Nothing stored, an equal or higher stored
        // version, or no app version set: `None`.
        let Some(app_version) = self.app_version.clone() else { return None };
        let stored = self.current_data_version()?;
        if app_info::cmp_versions(&stored, &app_version) == std::cmp::Ordering::Less {
            self.upgrade_claimed = true;
            Some(stored)
        } else {
            None
        }
    }

    fn mark_data_version(&mut self) {
        if let Some(version) = self.app_version.clone() {
            self.store_data_version(&version);
        }
    }
}

impl bindings::plinth::app::sql::Host for HostState {
    fn run(&mut self, kind: u8, statement: String, params: String) -> u32 {
        let id = self.requests.open();
        let results = self.net_results.clone();
        let answer = match self.policy.check(sql::STORE_SQL) {
            Err(reason) => Some(denied_text(reason).to_owned()),
            Ok(()) => match sql::Op::from_kind(kind, statement, params) {
                None => Some(format!("sql: unknown call kind {kind}")),
                Some(op) => match self.sql_store() {
                    None => Some(denied_text(DeniedReason::Unsupported).to_owned()),
                    Some(store) => {
                        store.submit(id, op, &results);
                        None
                    }
                },
            },
        };
        if let Some(error) = answer {
            results.lock().unwrap().push((id, sql::failure(error)));
        }
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

/// Which engine a `Runner` runs apps with (SPEC.md §9.2): wasmtime
/// (desktop, Android) or the wasmi interpreter (iOS; also
/// `PLINTH_ENGINE=wasmi` on desktop, for tests). `Runner::new` reads
/// `PLINTH_ENGINE` once and keeps the engine for its own lifetime.
enum Backend {
    #[cfg(feature = "wasmtime")]
    Wasmtime(wasmtime_engine::WasmtimeBackend),
    #[cfg(feature = "wasmi")]
    Wasmi { engine: wasmi::Engine },
}

/// True when `PLINTH_ENGINE=wasmi` (SPEC.md §9.2).
#[cfg(feature = "wasmtime")]
fn wants_wasmi() -> bool {
    std::env::var("PLINTH_ENGINE").map(|v| v == "wasmi").unwrap_or(false)
}

/// The engine is shared by all guests. The wasmtime backend owns the epoch
/// ticker thread.
pub struct Runner {
    backend: Backend,
}

impl Runner {
    /// Makes the engine: wasmtime on desktop and Android; wasmi when
    /// `PLINTH_ENGINE=wasmi`, and always in a build without the `wasmtime`
    /// feature.
    pub fn new() -> Result<Self> {
        #[cfg(feature = "wasmtime")]
        {
            if wants_wasmi() {
                #[cfg(feature = "wasmi")]
                return Ok(Self { backend: Backend::Wasmi { engine: wasmi_engine::new_engine() } });
                #[cfg(not(feature = "wasmi"))]
                bail!("PLINTH_ENGINE=wasmi, but this build has no `wasmi` feature (plinth-runner-wasmtime)");
            }
            Ok(Self { backend: Backend::Wasmtime(wasmtime_engine::WasmtimeBackend::new()?) })
        }
        #[cfg(not(feature = "wasmtime"))]
        {
            Ok(Self { backend: Backend::Wasmi { engine: wasmi_engine::new_engine() } })
        }
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
        let state = HostState::new(&limits, policy, kv, clipboard, hub);
        match &self.backend {
            #[cfg(feature = "wasmtime")]
            Backend::Wasmtime(backend) => backend.load(bytes, &limits, state).map(Guest::Wasmtime),
            #[cfg(feature = "wasmi")]
            Backend::Wasmi { engine } => wasmi_engine::load(engine, bytes, &limits, state).map(Guest::Wasmi),
        }
    }
}

/// One running app instance. An enum, not a trait object: the host logic
/// (`HostState`) is one type either way (SPEC.md §9.2); only how the guest
/// is called differs.
pub enum Guest {
    #[cfg(feature = "wasmtime")]
    Wasmtime(WasmtimeGuest),
    #[cfg(feature = "wasmi")]
    Wasmi(wasmi_engine::WasmiGuest),
}

impl Guest {
    fn data(&self) -> &HostState {
        match self {
            #[cfg(feature = "wasmtime")]
            Guest::Wasmtime(g) => g.store.data(),
            #[cfg(feature = "wasmi")]
            Guest::Wasmi(g) => g.store.data(),
        }
    }

    fn data_mut(&mut self) -> &mut HostState {
        match self {
            #[cfg(feature = "wasmtime")]
            Guest::Wasmtime(g) => g.store.data_mut(),
            #[cfg(feature = "wasmi")]
            Guest::Wasmi(g) => g.store.data_mut(),
        }
    }

    /// Returns the `dev.log` messages since the last call.
    pub fn take_logs(&mut self) -> Vec<String> {
        std::mem::take(&mut self.data_mut().logs)
    }

    /// The size of the guest's linear memory, in bytes. It only grows, so a
    /// long run that keeps it level has no leak (`tests/monkey.rs`).
    pub fn memory_bytes(&self) -> usize {
        self.data().limits.memory
    }

    /// The fuel that the last `init`/`on-event` call used on the wasmi
    /// engine (its time limit, `docs/GAPS.md` "wasmi engine"), for the
    /// calibration of the fuel budget. `None` on wasmtime (no fuel).
    pub fn last_call_fuel(&self) -> Option<u64> {
        match self {
            #[cfg(feature = "wasmtime")]
            Guest::Wasmtime(_) => None,
            #[cfg(feature = "wasmi")]
            Guest::Wasmi(g) => Some(g.last_fuel_used),
        }
    }

    /// Returns the uncaught errors that the guest reported (`error.report`,
    /// SPEC.md §5.6) since the last call. The guest is still running.
    pub fn take_errors(&mut self) -> Vec<String> {
        std::mem::take(&mut self.data_mut().errors)
    }

    /// Calls `init` and returns the op buffers that the guest committed.
    /// Calls `init` with the `init` records `args` (SPEC.md §8.1). Without a
    /// `RANDOM_SEED` record, it adds one with fresh entropy (core 1.12,
    /// `Math.random`); a test gives its own seed to repeat a run.
    pub fn init(&mut self, args: &[u8]) -> Result<Vec<Vec<u8>>> {
        use plinth_protocol::init_arg;
        let mut args = args.to_vec();
        if init_arg::find(&args, init_arg::RANDOM_SEED).is_none() {
            init_arg::push(&mut args, init_arg::RANDOM_SEED, &fresh_seed().to_le_bytes());
        }
        match self {
            #[cfg(feature = "wasmtime")]
            Guest::Wasmtime(g) => g.call_init(&args),
            #[cfg(feature = "wasmi")]
            Guest::Wasmi(_) => self.call_wasmi("init", &args),
        }
    }

    /// Calls `on-event` and returns the op buffers that the guest committed.
    pub fn on_event(&mut self, events: &[u8]) -> Result<Vec<Vec<u8>>> {
        match self {
            #[cfg(feature = "wasmtime")]
            Guest::Wasmtime(g) => g.call_on_event(events),
            #[cfg(feature = "wasmi")]
            Guest::Wasmi(_) => self.call_wasmi("on-event", events),
        }
    }

    /// Overrides `timezone-offset` with a fixed value (minutes east of
    /// UTC) for tests, instead of the real OS offset.
    pub fn set_fake_timezone_offset(&mut self, minutes: i32) {
        self.data_mut().fake_timezone_offset_minutes = Some(minutes);
    }

    /// Fixes the locale of the app (`plinth:locale`, core 1.13) to `tag`
    /// (normalized to BCP 47), instead of the OS locale: for tests, and for
    /// a host that lets the user choose a locale for each app. Sends no
    /// event; call it before `init`, or use `change_locale`.
    pub fn set_locale(&mut self, tag: &str) {
        let data = self.data_mut();
        data.locale = Some(locale::normalize(tag));
        data.locale_fixed = true;
    }

    /// Fixes the locale to `tag` (as `set_locale`) and, when it changed,
    /// sends the `lifecycle` event with kind `LOCALE` (`docs/I18N.md` §3.1).
    /// Returns the op buffers that the guest committed.
    pub fn change_locale(&mut self, tag: &str) -> Result<Vec<Vec<u8>>> {
        let before = self.data().locale.clone();
        self.set_locale(tag);
        if self.data().locale == before {
            return Ok(Vec::new());
        }
        self.on_event(&locale_event())
    }

    /// Reads the OS locale again. True when it changed since the guest last
    /// read it: the host then sends `locale_event()`. Always false when the
    /// locale is fixed (`set_locale`) or before the guest read it.
    pub fn refresh_locale(&mut self) -> bool {
        let data = self.data_mut();
        if data.locale_fixed || data.locale.is_none() {
            return false;
        }
        let now = locale::os_locale();
        if data.locale.as_deref() == Some(now.as_str()) {
            return false;
        }
        data.locale = Some(now);
        true
    }

    /// Sets the app's manifest version (`plinth:app-info`, core 1.13,
    /// `PRODUCTION.md` D4): `data-version()`/`mark-data-version()` and
    /// `finish_start` use it. Call it before `init`. Without it,
    /// `mark-data-version` and `finish_start` do nothing.
    pub fn set_app_version(&mut self, version: &str) {
        self.data_mut().app_version = Some(version.to_owned());
    }

    /// Called once `init` has returned (step B, `PRODUCTION.md` D4):
    /// stores the current app version as the data version, unless the
    /// version already stored is higher (a downgrade keeps the higher
    /// one) — UNLESS the guest already claimed a pending upgrade this run
    /// (`data-version()` returned a version), in which case storing is
    /// left to the guest's own `mark-data-version()` call. Does nothing
    /// when no app version was set (`set_app_version`).
    pub fn finish_start(&mut self) {
        let data = self.data_mut();
        if data.upgrade_claimed {
            return;
        }
        let Some(version) = data.app_version.clone() else { return };
        let stored = data.current_data_version();
        if app_info::should_store(stored.as_deref(), &version) {
            data.store_data_version(&version);
        }
    }

    /// Gives the guest its private space for `plinth:files` (core 1.11,
    /// `docs/STORAGE.md` §2, §3). Without it, every call is
    /// `denied:unsupported`. Call it before `init`.
    pub fn set_files(&mut self, files: files::Files) {
        self.data_mut().files = files;
    }

    /// Gives the guest the host's notification area for `plinth:notify`
    /// (core 1.12, `docs/PRODUCTION.md` H3). Without it, the guest has a
    /// `MemoryNotify`, which only records the calls. Call it before `init`.
    pub fn set_notify(&mut self, notify: Box<dyn NotifyBackend>) {
        self.data_mut().notify = notify;
    }

    /// Gives the guest the host's safe store for `plinth:secrets` (core 1.13,
    /// `docs/SECRETS.md`), and the id of the app that owns the secrets. The
    /// instance is `default`. Without it, every call is
    /// `denied:unsupported`. Call it before `init`.
    pub fn set_secrets(&mut self, backend: Box<dyn SecretsBackend>, app_id: &str) {
        let data = self.data_mut();
        data.secrets = backend;
        data.secrets_scope = SecretsScope::new(app_id);
    }

    /// Starts a test clock (`plinth test`, docs/TESTING.md): from now on,
    /// `time.now` is `epoch_ms`, `monotonic-now` stops, and timers are due
    /// only when `advance_test_clock` moves the clock on. Call it before
    /// `init`. Returns the instant of the clock.
    pub fn use_test_clock(&mut self, epoch_ms: u64) -> Instant {
        let data = self.data_mut();
        let now = Instant::now();
        data.monotonic_origin = now;
        data.test_clock = Some((now, epoch_ms));
        now
    }

    /// Moves the test clock to `to` (not back). Fires nothing: call
    /// `fire_due_timers` with the result. Returns the instant of the clock,
    /// or `None` without a test clock.
    pub fn advance_test_clock(&mut self, to: Instant) -> Option<Instant> {
        let data = self.data_mut();
        let (now, epoch) = data.test_clock?;
        if to > now {
            let ms = to.duration_since(now).as_millis() as u64;
            data.test_clock = Some((to, epoch + ms));
        }
        data.test_clock.map(|(n, _)| n)
    }

    /// The soonest time a timer is due, if any is pending. The desktop
    /// host sleeps until this instant (via `gpui`'s executor timers,
    /// SPEC.md §9.4), then calls `fire_due_timers`.
    pub fn next_timer_deadline(&self) -> Option<Instant> {
        self.data().timers.next_deadline()
    }

    /// Builds one `on-event` buffer with a `timer` event (protocol
    /// `Event::Timer`, SPEC.md §8.4) for every timer due at or before
    /// `now`, dispatches it, and returns the committed op buffers. Returns
    /// `Ok(vec![])` with no call if no timer is due.
    pub fn fire_due_timers(&mut self, now: Instant) -> Result<Vec<Vec<u8>>> {
        let due = self.data_mut().timers.due(now);
        if due.is_empty() {
            return Ok(Vec::new());
        }
        let mut w = plinth_protocol::Writer::new();
        for timer in due {
            w.event(&plinth_protocol::Event::Timer { timer });
        }
        self.on_event(w.as_bytes())
    }

    /// Whether the guest has a frame timer (core 1.12, `onFrame`). While it
    /// has one, the host draws frames without a pause and calls
    /// `fire_frame` before each frame.
    pub fn wants_frames(&self) -> bool {
        !self.data().timers.frames().is_empty()
    }

    /// Sends one `frame` event (protocol `Event::Frame`) to each frame
    /// timer, with the milliseconds since the previous frame (0 for the
    /// first), and returns the committed op buffers. Returns `Ok(vec![])`
    /// with no call if the guest has no frame timer.
    pub fn fire_frame(&mut self, now: Instant) -> Result<Vec<Vec<u8>>> {
        let data = self.data_mut();
        let frames = data.timers.frames();
        if frames.is_empty() {
            data.last_frame = None;
            return Ok(Vec::new());
        }
        let dt = data.last_frame.map_or(0.0, |last| now.saturating_duration_since(last).as_secs_f64() * 1000.0);
        data.last_frame = Some(now);
        let mut w = plinth_protocol::Writer::new();
        for timer in frames {
            w.event(&plinth_protocol::Event::Frame { timer, dt });
        }
        self.on_event(w.as_bytes())
    }

    /// The dialog requests the guest opened that are still waiting for an
    /// answer (SPEC.md §8.4, §8.5). The desktop host polls this the same
    /// way it polls timers; a test reads it to simulate the host answering.
    pub fn pending_dialogs(&self) -> &[PendingDialog] {
        &self.data().dialogs
    }

    /// The number of open requests that the host answers by itself (not
    /// dialogs, which wait for the user): `net`, `files`, `notify` and
    /// the others. `plinth test` waits for them in `settle`.
    pub fn open_host_requests(&self) -> usize {
        let data = self.data();
        data.requests.len().saturating_sub(data.dialogs.len())
    }

    /// Drains the `net.fetch` requests a worker thread has finished since
    /// the last call (SPEC.md §8.4, §8.5). The desktop host polls this the
    /// same way it polls timers and dialogs, and answers each one with
    /// `answer_dialog` (the same generic completion delivery).
    pub fn poll_net_results(&mut self) -> Vec<(u32, plinth_protocol::Value)> {
        std::mem::take(&mut *self.data().net_results.lock().unwrap())
    }

    /// Drains the `plinth:hub` `launch` requests the guest made since the
    /// last call (`docs/HUB.md` §4.1, §4.2). Empty when the guest has no
    /// `plinth:hub` backend. The host (`plinth-host-desktop`) polls this
    /// the same way it polls timers and dialogs, and opens a window with
    /// `open_app` for each id.
    pub fn take_hub_launches(&mut self) -> Vec<String> {
        match self.data_mut().hub.as_mut() {
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
        self.data_mut().requests.close(id);
        self.data_mut().dialogs.retain(|d| d.id != id);
        let mut w = plinth_protocol::Writer::new();
        w.event(&plinth_protocol::Event::Completion { request: id, result });
        self.on_event(w.as_bytes())
    }

    /// Calls `init`/`on-event` (`export` is `"init"` or `"on-event"`) on
    /// the wasmi backend. Only valid when `self` is `Guest::Wasmi`.
    // Without the `wasmtime` feature, `Guest` has one variant, so the `else`
    // below is unreachable; that is expected, not a bug.
    #[allow(irrefutable_let_patterns)]
    #[cfg(feature = "wasmi")]
    fn call_wasmi(&mut self, export: &str, bytes: &[u8]) -> Result<Vec<Vec<u8>>> {
        let Guest::Wasmi(g) = self else { unreachable!("call_wasmi on a non-wasmi guest") };
        if g.poisoned {
            bail!("the app stopped earlier");
        }
        let logs_before = g.store.data().logs.len();
        let result = wasmi_engine::call(g, export, bytes);
        let commits = std::mem::take(&mut g.store.data_mut().commits);
        if let Err(e) = result {
            g.poisoned = true;
            let reason = wasmi_engine::stop_reason(&e, &g.store.data().logs[logs_before..]);
            log::error!(target: "plinth::app", "the app stopped: {reason}: {e:?}");
            return Err(e.context(reason));
        }
        Ok(commits)
    }
}
