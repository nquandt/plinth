//! The wasmtime engine (SPEC.md §9.2): desktop and Android. It is behind the
//! `wasmtime` feature. A build for iOS does not have it (wasmtime needs a
//! JIT, which iOS does not allow), and runs apps on wasmi instead
//! (`wasmi_engine`). The host logic (`HostState`) is in `lib.rs` and is the
//! same for both engines.
//!
//! Each call has a time limit (epoch interruption) and the guest has a
//! memory cap.

use crate::bindings::App;
use crate::{HostState, Limiter, Limits};
use anyhow::{Context as _, Result, bail};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use wasmtime::component::{Component, HasSelf, Linker, types::ComponentItem};
use wasmtime::{Config, Engine, Store};

/// How often the epoch ticker increments the engine epoch.
const EPOCH_TICK: Duration = Duration::from_millis(10);

impl wasmtime::ResourceLimiter for Limiter {
    fn memory_growing(&mut self, _current: usize, desired: usize, maximum: Option<usize>) -> wasmtime::Result<bool> {
        let ok = crate::allow_growth(self.memory_cap, desired, maximum);
        if ok {
            self.memory = self.memory.max(desired);
        }
        Ok(ok)
    }
    fn memory_grow_failed(&mut self, _error: wasmtime::Error) -> wasmtime::Result<()> {
        Ok(())
    }
    fn table_growing(&mut self, _current: usize, _desired: usize, _maximum: Option<usize>) -> wasmtime::Result<bool> {
        Ok(true)
    }
    fn table_grow_failed(&mut self, _error: wasmtime::Error) -> wasmtime::Result<()> {
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

/// The wasmtime engine of a `Runner`. It owns the epoch ticker thread.
/// `last`: the last component that `load` compiled, with a hash of its
/// bytes. A host that loads the same app again (`plinth test` makes a new
/// instance for each test) does not compile it again.
pub struct WasmtimeBackend {
    engine: Engine,
    stop_ticker: Arc<AtomicBool>,
    last: std::sync::Mutex<Option<(u64, usize, Component)>>,
}

impl WasmtimeBackend {
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
        Ok(Self { engine, stop_ticker, last: std::sync::Mutex::new(None) })
    }

    /// Compiles a component, checks its imports, and instantiates it with
    /// `state` as the host state.
    pub fn load(&self, bytes: &[u8], limits: &Limits, state: HostState) -> Result<WasmtimeGuest> {
        let engine = &self.engine;
        let hash = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            bytes.hash(&mut h);
            h.finish()
        };
        let cached =
            self.last.lock().unwrap().as_ref().filter(|(h, len, _)| *h == hash && *len == bytes.len()).map(|(_, _, c)| c.clone());
        let component = match cached {
            Some(c) => c,
            None => {
                let c = Component::new(engine, bytes)
                    .map_err(anyhow::Error::from)
                    .context("the artifact is not a valid component")?;
                check_imports(engine, &c)?;
                *self.last.lock().unwrap() = Some((hash, bytes.len(), c.clone()));
                c
            }
        };

        let mut linker = Linker::<HostState>::new(engine);
        App::add_to_linker::<_, HasSelf<_>>(&mut linker, |s| s)?;

        let mut store = Store::new(engine, state);
        store.limiter(|s| &mut s.limits);
        store.epoch_deadline_trap();
        let ticks = (limits.call_timeout.as_millis() / EPOCH_TICK.as_millis()).max(1) as u64;
        store.set_epoch_deadline(ticks);
        let app = App::instantiate(&mut store, &component, &linker)
            .map_err(anyhow::Error::from)
            .context("instantiate the app")?;
        // The source map of the app code, for the place of a trap
        // (`docs/PRODUCTION.md` L9). An app built before it has none.
        let srcmap = plinth_link::srcmap::find(bytes);
        Ok(WasmtimeGuest { store, app, deadline_ticks: ticks, poisoned: false, srcmap })
    }
}

impl Drop for WasmtimeBackend {
    fn drop(&mut self) {
        self.stop_ticker.store(true, Ordering::Relaxed);
    }
}

/// Rejects any import outside the `plinth:app` world, `wasi:*` included.
fn check_imports(engine: &Engine, component: &Component) -> Result<()> {
    let ty = component.component_type();
    for (name, item) in ty.imports(engine) {
        if !name.starts_with(crate::ALLOWED_IMPORT_PREFIX) {
            bail!("the artifact imports `{name}`, which is not in the plinth:app world");
        }
        if !matches!(item.ty, ComponentItem::ComponentInstance(_)) {
            bail!("the artifact import `{name}` is not an interface");
        }
    }
    Ok(())
}

/// The wasmtime backend's own state, private so `HostState` (`pub(crate)`)
/// does not have to be `pub` just because `Guest` is.
pub struct WasmtimeGuest {
    pub(crate) store: Store<HostState>,
    app: App,
    deadline_ticks: u64,
    /// Set after a trap. A trapped instance is never called again.
    poisoned: bool,
    /// The source map of the linked module (`plinth_link::srcmap`), if the
    /// app has one.
    srcmap: Option<plinth_link::srcmap::SrcMap>,
}

impl WasmtimeGuest {
    /// Calls `init`/`on-event` and returns the op buffers that the guest
    /// committed.
    pub(crate) fn call(&mut self, f: impl FnOnce(&App, &mut Store<HostState>) -> wasmtime::Result<()>) -> Result<Vec<Vec<u8>>> {
        let WasmtimeGuest { store, app, deadline_ticks, poisoned, srcmap } = self;
        if *poisoned {
            bail!("the app stopped earlier");
        }
        store.set_epoch_deadline(*deadline_ticks);
        let logs_before = store.data().logs.len();
        let result = f(app, store);
        let commits = std::mem::take(&mut store.data_mut().commits);
        if let Err(e) = result {
            *poisoned = true;
            let mut reason = stop_reason(&e, &store.data().logs[logs_before..]);
            if let Some(place) = srcmap.as_ref().and_then(|m| trap_place(m, &e)) {
                reason = format!("{reason} ({place})");
            }
            log::error!(target: "plinth::app", "the app stopped: {reason}: {e:?}");
            return Err(anyhow::Error::from(e).context(reason));
        }
        Ok(commits)
    }

    pub(crate) fn call_init(&mut self, args: &[u8]) -> Result<Vec<Vec<u8>>> {
        self.call(|app, store| app.call_init(store, args))
    }

    pub(crate) fn call_on_event(&mut self, events: &[u8]) -> Result<Vec<Vec<u8>>> {
        self.call(|app, store| app.call_on_event(store, events))
    }
}

/// The source place (`app/main.tsx:12`) of the innermost app frame of a
/// trap, from the wasm backtrace and the source map. Frames of the core
/// and of the component's own small modules have no map entry.
fn trap_place(map: &plinth_link::srcmap::SrcMap, e: &wasmtime::Error) -> Option<String> {
    let bt = e.downcast_ref::<wasmtime::WasmBacktrace>()?;
    bt.frames().iter().find_map(|f| {
        let (file, line) = map.lookup(f.func_index(), u32::try_from(f.func_offset()?).ok()?)?;
        Some(format!("{file}:{line}"))
    })
}

/// A short, plain reason for a failed guest call, for the host's "This
/// app stopped" banner. The error keeps the wasm detail (`{e:#}`, `{e:?}`).
/// The core logs `trap: <reason>` before it traps (`plinth_rt::trap`), so
/// the last such log of the call is the best reason.
fn stop_reason(e: &wasmtime::Error, logs: &[String]) -> String {
    if let Some(msg) = logs.iter().rev().find_map(|l| l.strip_prefix("trap: ")) {
        return msg.to_owned();
    }
    match e.downcast_ref::<wasmtime::Trap>() {
        Some(wasmtime::Trap::Interrupt) => "the app is not responding (time limit reached)".into(),
        Some(wasmtime::Trap::StackOverflow) => "too many nested calls (stack overflow)".into(),
        Some(wasmtime::Trap::UnreachableCodeReached) => "a check in the app failed".into(),
        Some(wasmtime::Trap::IntegerDivisionByZero) => "an integer division by zero".into(),
        Some(wasmtime::Trap::MemoryOutOfBounds | wasmtime::Trap::HeapMisaligned | wasmtime::Trap::TableOutOfBounds) => {
            "an invalid memory access".into()
        }
        Some(t) => t.to_string(),
        None => "a host call failed".into(),
    }
}
