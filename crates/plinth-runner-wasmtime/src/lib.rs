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

pub mod kv;
pub mod policy;
pub mod timers;

use kv::Kv;
use policy::{DeniedReason, Policy};
use timers::TimerQueue;

mod bindings {
    wasmtime::component::bindgen!({
        path: "../../wit/plinth",
        world: "app",
    });
}

use bindings::App;
use bindings::plinth::app::error::{DeniedReason as WitDeniedReason, HostError};

/// Capability names (SPEC.md §11).
pub mod capability {
    pub const STORE_KV: &str = "store.kv";
    pub const CLIPBOARD_READ: &str = "clipboard.read";
    pub const CLIPBOARD_WRITE: &str = "clipboard.write";
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
    limits: StoreLimits,
    policy: Policy,
    kv: Kv,
    timers: TimerQueue,
    clipboard: Box<dyn Clipboard>,
    monotonic_origin: Instant,
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

impl bindings::plinth::app::error::Host for HostState {}

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
    /// policy and host API implementations (SPEC.md §9.4, §11).
    pub fn load_with_policy(
        &self,
        bytes: &[u8],
        limits: Limits,
        policy: Policy,
        kv: Kv,
        clipboard: Box<dyn Clipboard>,
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
            limits: StoreLimitsBuilder::new().memory_size(limits.memory_bytes).instances(4).build(),
            policy,
            kv,
            timers: TimerQueue::new(),
            clipboard,
            monotonic_origin: Instant::now(),
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

    /// Calls `init` and returns the op buffers that the guest committed.
    pub fn init(&mut self, args: &[u8]) -> Result<Vec<Vec<u8>>> {
        self.call(|app, store| app.call_init(store, args))
    }

    /// Calls `on-event` and returns the op buffers that the guest committed.
    pub fn on_event(&mut self, events: &[u8]) -> Result<Vec<Vec<u8>>> {
        self.call(|app, store| app.call_on_event(store, events))
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
