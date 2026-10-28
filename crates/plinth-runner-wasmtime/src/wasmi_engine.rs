//! The wasmi interpreter engine (SPEC.md §9.2): iOS, and desktop tests that
//! set `PLINTH_ENGINE=wasmi`.
//!
//! wasmi has no component model support, so this runs the linked core
//! module directly (`crate::wasmi_core::extract_core_module` pulls it out
//! of the component), with import shims at the flat core ABI: the same one
//! `web/plinth-web.js`'s `hostImports` implements by hand, and the one
//! `wit-bindgen`/wasmtime's canonical-ABI lowering produces for the
//! wasmtime path. Every shim calls the same `HostState` method the
//! wasmtime `bindings::...::Host` trait impls call, so the host logic
//! lives once, in `lib.rs`.
//!
//! Every interface is wired: `ui`, `dev`, `error`, `time`, `store`,
//! `clipboard`, `dialog`, `notify`, `net`, `files`, `sql` and `hub` (SPEC.md
//! §9.2 step 2).
//!
//! wasmi 2 is a register-machine interpreter (it compiles Wasm to its own
//! register-based bytecode ahead of a call, not a tree-walking
//! interpreter), which is why its speed is close to a simple bytecode VM
//! and far from wasmtime's compiled code; see `docs/GAPS.md` "wasmi
//! engine" for numbers.
//!
//! The call time limit (SPEC.md §9.2 step 3) uses fuel: wasmi has no
//! epoch-interruption primitive like wasmtime, but `Config::consume_fuel`
//! charges a fuel unit per executed Wasm instruction (roughly), and
//! `Store::set_fuel`/`get_fuel` read and refill it. `FUEL_PER_MS` converts
//! `Limits::call_timeout` to a fuel budget; its doc comment says how it
//! was measured. The memory cap reuses `crate::Limiter`,
//! through `wasmi_core::ResourceLimiter` (the same trait `wasmi::
//! ResourceLimiter` re-exports).

use crate::bindings::plinth::app::clipboard::Host as ClipboardHost;
use crate::bindings::plinth::app::dev::Host as DevHost;
use crate::bindings::plinth::app::dialog::Host as DialogHost;
use crate::bindings::plinth::app::error::{DeniedReason as WitDeniedReason, Host as ErrorHost, HostError};
use crate::bindings::plinth::app::files::Host as FilesHost;
use crate::bindings::plinth::app::sql::Host as SqlHost;
use crate::bindings::plinth::app::hub::Host as HubHost;
use crate::bindings::plinth::app::app_info::Host as AppInfoHost;
use crate::bindings::plinth::app::locale::Host as LocaleHost;
use crate::bindings::plinth::app::net::Host as NetHost;
use crate::bindings::plinth::app::notify::Host as NotifyHost;
use crate::bindings::plinth::app::secrets::Host as SecretsHost;
use crate::bindings::plinth::app::store::Host as StoreHost;
use crate::bindings::plinth::app::time::Host as TimeHost;
use crate::bindings::plinth::app::ui::Host as UiHost;
use crate::wasmi_core::extract_core_module;
use crate::{HostState, Limits};
use anyhow::{Context as _, Result};
use wasmi::{Caller, Config, Engine, Error as WasmiError, Extern, Linker, Memory, Module, Store, TypedFunc};

/// Fuel per millisecond of `Limits::call_timeout` (SPEC.md §9.2 step 3).
///
/// Measured with `tests/perf.rs` `engines_speed` (release build, the
/// owner's PC, 2026-10-08; `docs/GAPS.md` "wasmi engine"): real app work
/// uses 0.93 to 3.4 million fuel per ms. A 10,000-row List `init`, a Pong
/// frame, a Cells edit, strings, calls and a sort use 0.93 to 1.6 million;
/// a collection 2.3 million; a tight arithmetic loop 3.4 million. With 2
/// million, the 2 s limit stops a call after 1.2 s (the tight loop) to 4.3 s
/// (the sort), and after 2.5 s or more for usual UI work. Thus the limit
/// does not stop usual app work early, and it is in a 2x band of 2 s.
///
/// Fuel counts guest work, not time, so the budget is the same in each
/// build. A debug build of this crate runs wasmi about 100 times slower
/// (5,600 to 12,800 fuel per ms), so there a call can run for minutes
/// before it stops. Tests that hit the limit on purpose use a short
/// `call_timeout` on wasmi (`tests/errors.rs`).
const FUEL_PER_MS: u64 = 2_000_000;

/// Builds the wasmi engine, with fuel metering on (the call time limit,
/// SPEC.md §9.2 step 3; `FUEL_PER_MS` turns `Limits::call_timeout` into a
/// budget, set before each `init`/`on-event` call).
pub fn new_engine() -> Engine {
    let mut config = Config::default();
    config.consume_fuel(true);
    Engine::new(&config)
}

/// One running app instance on wasmi, the wasmi twin of `crate::Guest`'s
/// wasmtime fields.
pub struct WasmiGuest {
    pub(crate) store: Store<HostState>,
    memory: Memory,
    init_fn: TypedFunc<(i32, i32), ()>,
    on_event_fn: TypedFunc<(i32, i32), ()>,
    cabi_realloc: TypedFunc<(i32, i32, i32, i32), i32>,
    /// The fuel budget for one `init`/`on-event` call (SPEC.md §9.2 step 3,
    /// `FUEL_PER_MS`), set fresh before each call the same way the
    /// wasmtime path resets its epoch deadline.
    fuel_budget: u64,
    /// The fuel that the last `init`/`on-event` call used (for the
    /// calibration of `FUEL_PER_MS`, `docs/GAPS.md` "wasmi engine").
    pub(crate) last_fuel_used: u64,
    /// Set after a trap. A trapped instance is never called again.
    pub(crate) poisoned: bool,
}

/// Extracts the core module, links the host imports, and instantiates it
/// (the wasmi twin of `Runner::load_with_policy_and_hub`).
pub fn load(
    engine: &Engine,
    bytes: &[u8],
    limits: &Limits,
    state: HostState,
) -> Result<WasmiGuest> {
    let core = extract_core_module(bytes).context("extract the linked core module")?;
    let module = Module::new(engine, &core).context("the core module is not valid wasm")?;

    let mut linker = Linker::<HostState>::new(engine);
    register_imports(&mut linker)?;

    let mut store = Store::new(engine, state);
    store.limiter(|s| &mut s.limits);
    let fuel_budget = (limits.call_timeout.as_millis() as u64).saturating_mul(FUEL_PER_MS).max(1);
    // Instantiation itself runs Wasm (active element segments, globals):
    // give it a generous one-off budget, separate from the per-call
    // budget below.
    store.set_fuel(fuel_budget.max(10_000_000)).map_err(anyhow::Error::from)?;
    let instance =
        linker.instantiate_and_start(&mut store, &module).map_err(anyhow::Error::from).context("instantiate the app")?;

    let memory = instance.get_memory(&store, "memory").context("the core module has no memory export")?;
    let init_fn = instance
        .get_typed_func::<(i32, i32), ()>(&store, "init")
        .map_err(anyhow::Error::from)
        .context("the core module has no init export")?;
    let on_event_fn = instance
        .get_typed_func::<(i32, i32), ()>(&store, "on-event")
        .map_err(anyhow::Error::from)
        .context("the core module has no on-event export")?;
    let cabi_realloc = instance
        .get_typed_func::<(i32, i32, i32, i32), i32>(&store, "cabi_realloc")
        .map_err(anyhow::Error::from)
        .context("the core module has no cabi_realloc export")?;

    Ok(WasmiGuest { store, memory, init_fn, on_event_fn, cabi_realloc, fuel_budget, last_fuel_used: 0, poisoned: false })
}

/// Calls `init` or `on-event` with `bytes` as its `list<u8>` argument: the
/// same buffer passing wasmtime/bindgen does, by hand (`cabi_realloc` in
/// the guest, then a plain `(ptr, len)` call). Resets the fuel budget
/// first (SPEC.md §9.2 step 3), the same way the wasmtime path resets its
/// epoch deadline before each call.
pub fn call(g: &mut WasmiGuest, export: &str, bytes: &[u8]) -> Result<()> {
    g.store.set_fuel(g.fuel_budget).map_err(anyhow::Error::from)?;
    let ptr = g.cabi_realloc.call(&mut g.store, (0, 0, 1, bytes.len().max(1) as i32)).map_err(anyhow::Error::from)?;
    g.memory.write(&mut g.store, ptr as usize, bytes).map_err(|e| anyhow::anyhow!("write the guest buffer: {e:#}"))?;
    let f = if export == "init" { &g.init_fn } else { &g.on_event_fn };
    let result = f.call(&mut g.store, (ptr, bytes.len() as i32)).map_err(anyhow::Error::from);
    g.last_fuel_used = g.fuel_budget - g.store.get_fuel().unwrap_or(0);
    result
}

/// A short, plain reason for a failed guest call, the wasmi twin of
/// `crate::stop_reason`.
pub fn stop_reason(e: &anyhow::Error, logs: &[String]) -> String {
    if let Some(msg) = logs.iter().rev().find_map(|l| l.strip_prefix("trap: ")) {
        return msg.to_owned();
    }
    if let Some(we) = e.downcast_ref::<WasmiError>() {
        if let Some(code) = we.as_trap_code() {
            return match code {
                wasmi::TrapCode::OutOfFuel => "the app is not responding (time limit reached)".into(),
                wasmi::TrapCode::UnreachableCodeReached => "a check in the app failed".into(),
                wasmi::TrapCode::IntegerDivisionByZero => "an integer division by zero".into(),
                wasmi::TrapCode::MemoryOutOfBounds | wasmi::TrapCode::TableOutOfBounds => "an invalid memory access".into(),
                wasmi::TrapCode::StackOverflow => "too many nested calls (stack overflow)".into(),
                other => other.to_string(),
            };
        }
        return we.to_string();
    }
    "a host call failed".into()
}

// -- Memory and canonical-ABI result helpers (match `web/plinth-web.js`'s
// `hostImports`'s `read*`/`write*` helpers byte for byte). ------------------

fn memory(caller: &Caller<'_, HostState>) -> Memory {
    caller.get_export("memory").and_then(Extern::into_memory).expect("the core module exports memory")
}

fn read_bytes(caller: &mut Caller<'_, HostState>, ptr: i32, len: i32) -> Result<Vec<u8>, WasmiError> {
    let mem = memory(caller);
    let mut buf = vec![0u8; len.max(0) as usize];
    mem.read(&*caller, ptr as usize, &mut buf).map_err(|e| WasmiError::new(e.to_string()))?;
    Ok(buf)
}

fn read_string(caller: &mut Caller<'_, HostState>, ptr: i32, len: i32) -> Result<String, WasmiError> {
    Ok(String::from_utf8_lossy(&read_bytes(caller, ptr, len)?).into_owned())
}

/// Reads one little-endian `u32` out of guest memory (an element of a
/// `headers: list<tuple<string, string>>` record, for `net.fetch`).
fn read_u32(caller: &mut Caller<'_, HostState>, ptr: i32) -> Result<u32, WasmiError> {
    let bytes = read_bytes(caller, ptr, 4)?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}

fn write_bytes(caller: &mut Caller<'_, HostState>, ptr: i32, data: &[u8]) -> Result<(), WasmiError> {
    let mem = memory(caller);
    mem.write(&mut *caller, ptr as usize, data).map_err(|e| WasmiError::new(e.to_string()))
}

/// Allocates `len` bytes in the guest's memory via its own `cabi_realloc`
/// (the caller-allocates-the-result convention the canonical ABI uses for
/// a `list<u8>`/`string` a host call returns).
fn alloc(caller: &mut Caller<'_, HostState>, len: usize) -> Result<i32, WasmiError> {
    let f = caller.get_export("cabi_realloc").and_then(Extern::into_func).expect("the core module exports cabi_realloc");
    let typed: TypedFunc<(i32, i32, i32, i32), i32> = f.typed(&*caller)?;
    typed.call(&mut *caller, (0, 0, 1, len.max(1) as i32))
}

fn write_string(caller: &mut Caller<'_, HostState>, s: &str) -> Result<(i32, i32), WasmiError> {
    let bytes = s.as_bytes();
    let ptr = alloc(caller, bytes.len())?;
    write_bytes(caller, ptr, bytes)?;
    Ok((ptr, bytes.len() as i32))
}

/// `result<_, host-error>`: tag@0 (1 byte); err payload (host-error:
/// tag@1, reason@2).
fn write_ok_unit(caller: &mut Caller<'_, HostState>, retptr: i32) -> Result<(), WasmiError> {
    write_bytes(caller, retptr, &[0])
}
fn write_denied_unit(caller: &mut Caller<'_, HostState>, retptr: i32, reason: u8) -> Result<(), WasmiError> {
    write_bytes(caller, retptr, &[1, 0, reason])
}

/// `result<T, host-error>` where `T` contains an i32 (`option<string>`,
/// `list<string>`, `u32`): outer align is 4, so tag@0 and the payload (ok
/// or err) start at offset 4.
fn write_denied_at4(caller: &mut Caller<'_, HostState>, retptr: i32, reason: u8) -> Result<(), WasmiError> {
    write_bytes(caller, retptr, &[1])?;
    write_bytes(caller, retptr + 4, &[0, reason])
}
fn write_ok_u32(caller: &mut Caller<'_, HostState>, retptr: i32, value: u32) -> Result<(), WasmiError> {
    write_bytes(caller, retptr, &[0])?;
    write_bytes(caller, retptr + 4, &value.to_le_bytes())
}
/// `Ok(option<string>)`: tag@0=0, option-tag@4 (0=none,1=some), string
/// ptr@8/len@12 if some.
fn write_ok_option_string(caller: &mut Caller<'_, HostState>, retptr: i32, value: Option<&str>) -> Result<(), WasmiError> {
    write_bytes(caller, retptr, &[0])?;
    match value {
        None => write_bytes(caller, retptr + 4, &[0]),
        Some(s) => {
            write_bytes(caller, retptr + 4, &[1])?;
            let (ptr, len) = write_string(caller, s)?;
            write_bytes(caller, retptr + 8, &ptr.to_le_bytes())?;
            write_bytes(caller, retptr + 12, &len.to_le_bytes())
        }
    }
}
/// A plain `option<string>` (not wrapped in a `result`, `plinth:app-info`,
/// core 1.13): option-tag@0 (0=none,1=some), string ptr@4/len@8 if some.
fn write_option_string(caller: &mut Caller<'_, HostState>, retptr: i32, value: Option<&str>) -> Result<(), WasmiError> {
    match value {
        None => write_bytes(caller, retptr, &[0]),
        Some(s) => {
            write_bytes(caller, retptr, &[1])?;
            let (ptr, len) = write_string(caller, s)?;
            write_bytes(caller, retptr + 4, &ptr.to_le_bytes())?;
            write_bytes(caller, retptr + 8, &len.to_le_bytes())
        }
    }
}
/// `Ok(string)` (not an `option`): tag@0=0, ptr@4/len@8. A plain string's
/// natural alignment is already 4, so there is no option-tag byte.
fn write_ok_string(caller: &mut Caller<'_, HostState>, retptr: i32, value: &str) -> Result<(), WasmiError> {
    write_bytes(caller, retptr, &[0])?;
    let (ptr, len) = write_string(caller, value)?;
    write_bytes(caller, retptr + 4, &ptr.to_le_bytes())?;
    write_bytes(caller, retptr + 8, &len.to_le_bytes())
}
/// `Ok(list<string>)`: tag@0=0, list ptr@4/len@8; each element is (ptr:
/// i32, len: i32).
fn write_ok_string_list(caller: &mut Caller<'_, HostState>, retptr: i32, values: &[String]) -> Result<(), WasmiError> {
    write_bytes(caller, retptr, &[0])?;
    let arr_ptr = alloc(caller, values.len().max(1) * 8)?;
    for (i, v) in values.iter().enumerate() {
        let (ptr, len) = write_string(caller, v)?;
        let base = arr_ptr + (i as i32) * 8;
        write_bytes(caller, base, &ptr.to_le_bytes())?;
        write_bytes(caller, base + 4, &len.to_le_bytes())?;
    }
    write_bytes(caller, retptr + 4, &arr_ptr.to_le_bytes())?;
    write_bytes(caller, retptr + 8, &(values.len() as i32).to_le_bytes())
}

/// The reason byte `wit/plinth/app.wit`'s `denied-reason` enum encodes
/// (`undeclared` = 0, `refused` = 1, `unsupported` = 2), from a
/// `HostError`.
fn reason_byte(e: HostError) -> u8 {
    match e {
        HostError::Denied(WitDeniedReason::Undeclared) => 0,
        HostError::Denied(WitDeniedReason::Refused) => 1,
        HostError::Denied(WitDeniedReason::Unsupported) => 2,
    }
}

/// `dialog`/`net`/`files`/`hub` (SPEC.md §9.2 step 2): real shims, each
/// calling the same `HostState` method the wasmtime `bindings::...::Host`
/// trait impls call. Layouts match `web/plinth-web.js`'s `hostImports`.
fn register_imports(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap("plinth:app/ui@1.0.0", "commit", |mut caller: Caller<HostState>, ptr: i32, len: i32| {
        let bytes = read_bytes(&mut caller, ptr, len)?;
        UiHost::commit(caller.data_mut(), bytes);
        Ok::<(), WasmiError>(())
    })?;

    linker.func_wrap("plinth:app/dev@1.0.0", "log", |mut caller: Caller<HostState>, ptr: i32, len: i32| {
        let msg = read_string(&mut caller, ptr, len)?;
        DevHost::log(caller.data_mut(), msg);
        Ok::<(), WasmiError>(())
    })?;

    linker.func_wrap("plinth:app/error@1.0.0", "report", |mut caller: Caller<HostState>, ptr: i32, len: i32| {
        let msg = read_string(&mut caller, ptr, len)?;
        ErrorHost::report(caller.data_mut(), msg);
        Ok::<(), WasmiError>(())
    })?;

    linker.func_wrap("plinth:app/time@1.0.0", "now", |mut caller: Caller<HostState>| -> i64 {
        TimeHost::now(caller.data_mut()) as i64
    })?;
    linker.func_wrap("plinth:app/time@1.0.0", "monotonic-now", |mut caller: Caller<HostState>| -> i64 {
        TimeHost::monotonic_now(caller.data_mut()) as i64
    })?;
    linker.func_wrap(
        "plinth:app/time@1.0.0",
        "set-timer",
        |mut caller: Caller<HostState>, ms: i32, repeat: i32, retptr: i32| {
            match TimeHost::set_timer(caller.data_mut(), ms as u32, repeat != 0) {
                Ok(id) => write_ok_u32(&mut caller, retptr, id),
                Err(e) => write_denied_at4(&mut caller, retptr, reason_byte(e)),
            }
        },
    )?;
    linker.func_wrap("plinth:app/time@1.0.0", "cancel-timer", |mut caller: Caller<HostState>, id: i32| {
        TimeHost::cancel_timer(caller.data_mut(), id as u32);
    })?;
    linker.func_wrap("plinth:app/time@1.0.0", "set-frame-timer", |mut caller: Caller<HostState>, retptr: i32| {
        match TimeHost::set_frame_timer(caller.data_mut()) {
            Ok(id) => write_ok_u32(&mut caller, retptr, id),
            Err(e) => write_denied_at4(&mut caller, retptr, reason_byte(e)),
        }
    })?;
    linker.func_wrap("plinth:app/time@1.0.0", "timezone-offset", |mut caller: Caller<HostState>, ms: i64| -> i32 {
        TimeHost::timezone_offset(caller.data_mut(), ms)
    })?;

    // `plinth:locale` (core 1.13): `tag` returns a plain string through
    // `retptr` (ptr@0, len@4); `is-rtl` returns a bool as an i32.
    linker.func_wrap("plinth:app/locale@1.0.0", "tag", |mut caller: Caller<HostState>, retptr: i32| {
        let tag = LocaleHost::tag(caller.data_mut());
        let (ptr, len) = write_string(&mut caller, &tag)?;
        write_bytes(&mut caller, retptr, &ptr.to_le_bytes())?;
        write_bytes(&mut caller, retptr + 4, &len.to_le_bytes())
    })?;
    linker.func_wrap("plinth:app/locale@1.0.0", "is-rtl", |mut caller: Caller<HostState>| -> i32 {
        i32::from(LocaleHost::is_rtl(caller.data_mut()))
    })?;
    // Formatting (docs/I18N.md §3.7): string parameters come as (ptr, len);
    // a string result goes through `retptr` (ptr@0, len@4).
    fn ret_string(caller: &mut Caller<'_, HostState>, retptr: i32, s: &str) -> Result<(), WasmiError> {
        let (ptr, len) = write_string(caller, s)?;
        write_bytes(caller, retptr, &ptr.to_le_bytes())?;
        write_bytes(caller, retptr + 4, &len.to_le_bytes())
    }
    linker.func_wrap(
        "plinth:app/locale@1.0.0",
        "format-number",
        |mut caller: Caller<HostState>, value: f64, o_ptr: i32, o_len: i32, retptr: i32| {
            let options = read_string(&mut caller, o_ptr, o_len)?;
            let text = LocaleHost::format_number(caller.data_mut(), value, options);
            ret_string(&mut caller, retptr, &text)
        },
    )?;
    linker.func_wrap(
        "plinth:app/locale@1.0.0",
        "format-date",
        |mut caller: Caller<HostState>, ms: f64, o_ptr: i32, o_len: i32, retptr: i32| {
            let options = read_string(&mut caller, o_ptr, o_len)?;
            let text = LocaleHost::format_date(caller.data_mut(), ms, options);
            ret_string(&mut caller, retptr, &text)
        },
    )?;
    linker.func_wrap(
        "plinth:app/locale@1.0.0",
        "plural-category",
        |mut caller: Caller<HostState>, value: f64, k_ptr: i32, k_len: i32, retptr: i32| {
            let kind = read_string(&mut caller, k_ptr, k_len)?;
            let text = LocaleHost::plural_category(caller.data_mut(), value, kind);
            ret_string(&mut caller, retptr, &text)
        },
    )?;
    linker.func_wrap(
        "plinth:app/locale@1.0.0",
        "format-list",
        |mut caller: Caller<HostState>, i_ptr: i32, i_len: i32, k_ptr: i32, k_len: i32, retptr: i32| {
            let items = read_string(&mut caller, i_ptr, i_len)?;
            let kind = read_string(&mut caller, k_ptr, k_len)?;
            let text = LocaleHost::format_list(caller.data_mut(), items, kind);
            ret_string(&mut caller, retptr, &text)
        },
    )?;
    linker.func_wrap(
        "plinth:app/locale@1.0.0",
        "compare",
        |mut caller: Caller<HostState>, a_ptr: i32, a_len: i32, b_ptr: i32, b_len: i32| -> Result<i32, WasmiError> {
            let a = read_string(&mut caller, a_ptr, a_len)?;
            let b = read_string(&mut caller, b_ptr, b_len)?;
            Ok(LocaleHost::compare(caller.data_mut(), a, b))
        },
    )?;

    // `plinth:app-info` (core 1.13, `PRODUCTION.md` D4): `data-version`
    // returns a plain `option<string>` through `retptr` (tag@0, ptr@4,
    // len@8); `mark-data-version` takes and returns nothing.
    linker.func_wrap("plinth:app/app-info@1.0.0", "data-version", |mut caller: Caller<HostState>, retptr: i32| {
        let version = AppInfoHost::data_version(caller.data_mut());
        write_option_string(&mut caller, retptr, version.as_deref())
    })?;
    linker.func_wrap("plinth:app/app-info@1.0.0", "mark-data-version", |mut caller: Caller<HostState>| {
        AppInfoHost::mark_data_version(caller.data_mut());
    })?;

    linker.func_wrap(
        "plinth:app/store@1.0.0",
        "kv-get",
        |mut caller: Caller<HostState>, key_ptr: i32, key_len: i32, retptr: i32| {
            let key = read_string(&mut caller, key_ptr, key_len)?;
            match StoreHost::kv_get(caller.data_mut(), key) {
                Ok(v) => write_ok_option_string(&mut caller, retptr, v.as_deref()),
                Err(e) => write_denied_at4(&mut caller, retptr, reason_byte(e)),
            }
        },
    )?;
    linker.func_wrap(
        "plinth:app/store@1.0.0",
        "kv-set",
        |mut caller: Caller<HostState>, key_ptr: i32, key_len: i32, val_ptr: i32, val_len: i32, retptr: i32| {
            let key = read_string(&mut caller, key_ptr, key_len)?;
            let value = read_string(&mut caller, val_ptr, val_len)?;
            match StoreHost::kv_set(caller.data_mut(), key, value) {
                Ok(()) => write_ok_unit(&mut caller, retptr),
                Err(e) => write_denied_unit(&mut caller, retptr, reason_byte(e)),
            }
        },
    )?;
    linker.func_wrap(
        "plinth:app/store@1.0.0",
        "kv-delete",
        |mut caller: Caller<HostState>, key_ptr: i32, key_len: i32, retptr: i32| {
            let key = read_string(&mut caller, key_ptr, key_len)?;
            match StoreHost::kv_delete(caller.data_mut(), key) {
                Ok(()) => write_ok_unit(&mut caller, retptr),
                Err(e) => write_denied_unit(&mut caller, retptr, reason_byte(e)),
            }
        },
    )?;
    linker.func_wrap("plinth:app/store@1.0.0", "kv-keys", |mut caller: Caller<HostState>, retptr: i32| {
        match StoreHost::kv_keys(caller.data_mut()) {
            Ok(keys) => write_ok_string_list(&mut caller, retptr, &keys),
            Err(e) => write_denied_at4(&mut caller, retptr, reason_byte(e)),
        }
    })?;

    linker.func_wrap(
        "plinth:app/clipboard@1.0.0",
        "write-text",
        |mut caller: Caller<HostState>, ptr: i32, len: i32, retptr: i32| {
            let text = read_string(&mut caller, ptr, len)?;
            match ClipboardHost::write_text(caller.data_mut(), text) {
                Ok(()) => write_ok_unit(&mut caller, retptr),
                Err(e) => write_denied_unit(&mut caller, retptr, reason_byte(e)),
            }
        },
    )?;
    linker.func_wrap("plinth:app/clipboard@1.0.0", "read-text", |mut caller: Caller<HostState>, retptr: i32| {
        match ClipboardHost::read_text(caller.data_mut()) {
            Ok(v) => write_ok_option_string(&mut caller, retptr, v.as_deref()),
            Err(e) => write_denied_at4(&mut caller, retptr, reason_byte(e)),
        }
    })?;

    linker.func_wrap("plinth:app/dialog@1.0.0", "alert", |mut caller: Caller<HostState>, ptr: i32, len: i32| -> Result<i32, WasmiError> {
        let message = read_string(&mut caller, ptr, len)?;
        Ok(DialogHost::alert(caller.data_mut(), message) as i32)
    })?;
    linker.func_wrap("plinth:app/dialog@1.0.0", "confirm", |mut caller: Caller<HostState>, ptr: i32, len: i32| -> Result<i32, WasmiError> {
        let message = read_string(&mut caller, ptr, len)?;
        Ok(DialogHost::confirm(caller.data_mut(), message) as i32)
    })?;
    linker.func_wrap("plinth:app/dialog@1.0.0", "prompt", |mut caller: Caller<HostState>, ptr: i32, len: i32| -> Result<i32, WasmiError> {
        let message = read_string(&mut caller, ptr, len)?;
        Ok(DialogHost::prompt(caller.data_mut(), message) as i32)
    })?;

    // `notify.show` (core 1.12): title(ptr,len), body(tag,ptr,len),
    // tag(tag,ptr,len) -- each `option<string>` lowers like `net.fetch`'s
    // `body` (tag 0 = none). Returns the request id directly.
    linker.func_wrap(
        "plinth:app/notify@1.0.0",
        "show",
        |mut caller: Caller<HostState>,
         title_ptr: i32,
         title_len: i32,
         body_tag: i32,
         body_ptr: i32,
         body_len: i32,
         tag_tag: i32,
         tag_ptr: i32,
         tag_len: i32|
         -> Result<i32, WasmiError> {
            let title = read_string(&mut caller, title_ptr, title_len)?;
            let body = if body_tag == 0 { None } else { Some(read_string(&mut caller, body_ptr, body_len)?) };
            let tag = if tag_tag == 0 { None } else { Some(read_string(&mut caller, tag_ptr, tag_len)?) };
            Ok(NotifyHost::show(caller.data_mut(), title, body, tag) as i32)
        },
    )?;
    linker.func_wrap(
        "plinth:app/notify@1.0.0",
        "cancel",
        |mut caller: Caller<HostState>, ptr: i32, len: i32, retptr: i32| {
            let tag = read_string(&mut caller, ptr, len)?;
            match NotifyHost::cancel(caller.data_mut(), tag) {
                Ok(()) => write_ok_unit(&mut caller, retptr),
                Err(e) => write_denied_unit(&mut caller, retptr, reason_byte(e)),
            }
        },
    )?;

    // `secrets` (core 1.13): `set(name, value)`, `get(name)`, `remove(name)`
    // and `names()`. Each returns the request id directly.
    linker.func_wrap(
        "plinth:app/secrets@1.0.0",
        "set",
        |mut caller: Caller<HostState>, n_ptr: i32, n_len: i32, v_ptr: i32, v_len: i32| -> Result<i32, WasmiError> {
            let name = read_string(&mut caller, n_ptr, n_len)?;
            let value = read_string(&mut caller, v_ptr, v_len)?;
            Ok(SecretsHost::set(caller.data_mut(), name, value) as i32)
        },
    )?;
    linker.func_wrap("plinth:app/secrets@1.0.0", "get", |mut caller: Caller<HostState>, ptr: i32, len: i32| -> Result<i32, WasmiError> {
        let name = read_string(&mut caller, ptr, len)?;
        Ok(SecretsHost::get(caller.data_mut(), name) as i32)
    })?;
    linker.func_wrap("plinth:app/secrets@1.0.0", "remove", |mut caller: Caller<HostState>, ptr: i32, len: i32| -> Result<i32, WasmiError> {
        let name = read_string(&mut caller, ptr, len)?;
        Ok(SecretsHost::remove(caller.data_mut(), name) as i32)
    })?;
    linker.func_wrap("plinth:app/secrets@1.0.0", "names", |mut caller: Caller<HostState>| -> Result<i32, WasmiError> {
        Ok(SecretsHost::names(caller.data_mut()) as i32)
    })?;

    // `net.fetch` (SPEC.md §8.5, §11): url(ptr,len), method(ptr,len),
    // headers(ptr,count) -- each element a 16-byte (ptr,len,ptr,len)
    // tuple<string,string> -- body(tag,ptr,len) (an `option<string>`, tag
    // 0 = none). Same lowered core ABI as `web/plinth-web.js`'s `fetch`.
    linker.func_wrap(
        "plinth:app/net@1.0.0",
        "fetch",
        |mut caller: Caller<HostState>,
         url_ptr: i32,
         url_len: i32,
         method_ptr: i32,
         method_len: i32,
         headers_ptr: i32,
         headers_count: i32,
         body_tag: i32,
         body_ptr: i32,
         body_len: i32|
         -> Result<i32, WasmiError> {
            let url = read_string(&mut caller, url_ptr, url_len)?;
            let method = read_string(&mut caller, method_ptr, method_len)?;
            let mut headers = Vec::with_capacity(headers_count.max(0) as usize);
            for i in 0..headers_count {
                let base = headers_ptr + i * 16;
                let (k_ptr, k_len) = (read_u32(&mut caller, base)? as i32, read_u32(&mut caller, base + 4)? as i32);
                let (v_ptr, v_len) = (read_u32(&mut caller, base + 8)? as i32, read_u32(&mut caller, base + 12)? as i32);
                headers.push((read_string(&mut caller, k_ptr, k_len)?, read_string(&mut caller, v_ptr, v_len)?));
            }
            let body = if body_tag == 0 { None } else { Some(read_string(&mut caller, body_ptr, body_len)?) };
            Ok(NetHost::fetch(caller.data_mut(), url, method, headers, body) as i32)
        },
    )?;

    linker.func_wrap("plinth:app/files@1.0.0", "read", |mut caller: Caller<HostState>, ptr: i32, len: i32| -> Result<i32, WasmiError> {
        let path = read_string(&mut caller, ptr, len)?;
        Ok(FilesHost::read(caller.data_mut(), path) as i32)
    })?;
    linker.func_wrap(
        "plinth:app/files@1.0.0",
        "write",
        |mut caller: Caller<HostState>, ptr: i32, len: i32, text_ptr: i32, text_len: i32| -> Result<i32, WasmiError> {
            let path = read_string(&mut caller, ptr, len)?;
            let text = read_string(&mut caller, text_ptr, text_len)?;
            Ok(FilesHost::write(caller.data_mut(), path, text) as i32)
        },
    )?;
    linker.func_wrap("plinth:app/files@1.0.0", "list-dir", |mut caller: Caller<HostState>, ptr: i32, len: i32| -> Result<i32, WasmiError> {
        let dir = read_string(&mut caller, ptr, len)?;
        Ok(FilesHost::list_dir(caller.data_mut(), dir) as i32)
    })?;
    linker.func_wrap("plinth:app/files@1.0.0", "stat", |mut caller: Caller<HostState>, ptr: i32, len: i32| -> Result<i32, WasmiError> {
        let path = read_string(&mut caller, ptr, len)?;
        Ok(FilesHost::stat(caller.data_mut(), path) as i32)
    })?;
    linker.func_wrap("plinth:app/files@1.0.0", "remove", |mut caller: Caller<HostState>, ptr: i32, len: i32| -> Result<i32, WasmiError> {
        let path = read_string(&mut caller, ptr, len)?;
        Ok(FilesHost::remove(caller.data_mut(), path) as i32)
    })?;
    // store.sql (core 1.13): `run(kind, statement, params)`. The `u8` kind is
    // an `i32` in the core ABI.
    linker.func_wrap(
        "plinth:app/sql@1.0.0",
        "run",
        |mut caller: Caller<HostState>, kind: i32, s_ptr: i32, s_len: i32, p_ptr: i32, p_len: i32| -> Result<i32, WasmiError> {
            let statement = read_string(&mut caller, s_ptr, s_len)?;
            let params = read_string(&mut caller, p_ptr, p_len)?;
            Ok(SqlHost::run(caller.data_mut(), kind as u8, statement, params) as i32)
        },
    )?;

    register_hub_imports(linker)?;
    Ok(())
}

/// `plinth:hub` (`docs/HUB.md` §4.1, §12.2, §12.3): privileged library and
/// Hub management calls. Most return `result<_, host-error>` or
/// `result<string, host-error>` through `retptr`; `search`, `install`,
/// `check-updates` and `update` are async (a request id now, SPEC.md
/// §8.4/§8.5) and return `u32` directly.
fn register_hub_imports(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap("plinth:app/hub@1.0.0", "list-apps", |mut caller: Caller<HostState>, retptr: i32| {
        match HubHost::list_apps(caller.data_mut()) {
            Ok(json) => write_ok_string(&mut caller, retptr, &json),
            Err(e) => write_denied_at4(&mut caller, retptr, reason_byte(e)),
        }
    })?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "launch",
        |mut caller: Caller<HostState>, id_ptr: i32, id_len: i32, retptr: i32| {
            let id = read_string(&mut caller, id_ptr, id_len)?;
            match HubHost::launch(caller.data_mut(), id) {
                Ok(()) => write_ok_unit(&mut caller, retptr),
                Err(e) => write_denied_unit(&mut caller, retptr, reason_byte(e)),
            }
        },
    )?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "set-grant",
        |mut caller: Caller<HostState>, id_ptr: i32, id_len: i32, cap_ptr: i32, cap_len: i32, allowed: i32, retptr: i32| {
            let id = read_string(&mut caller, id_ptr, id_len)?;
            let capability = read_string(&mut caller, cap_ptr, cap_len)?;
            match HubHost::set_grant(caller.data_mut(), id, capability, allowed != 0) {
                Ok(()) => write_ok_unit(&mut caller, retptr),
                Err(e) => write_denied_unit(&mut caller, retptr, reason_byte(e)),
            }
        },
    )?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "block",
        |mut caller: Caller<HostState>, id_ptr: i32, id_len: i32, retptr: i32| {
            let id = read_string(&mut caller, id_ptr, id_len)?;
            match HubHost::block(caller.data_mut(), id) {
                Ok(()) => write_ok_unit(&mut caller, retptr),
                Err(e) => write_denied_unit(&mut caller, retptr, reason_byte(e)),
            }
        },
    )?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "unblock",
        |mut caller: Caller<HostState>, id_ptr: i32, id_len: i32, retptr: i32| {
            let id = read_string(&mut caller, id_ptr, id_len)?;
            match HubHost::unblock(caller.data_mut(), id) {
                Ok(()) => write_ok_unit(&mut caller, retptr),
                Err(e) => write_denied_unit(&mut caller, retptr, reason_byte(e)),
            }
        },
    )?;
    linker.func_wrap("plinth:app/hub@1.0.0", "list-groups", |mut caller: Caller<HostState>, retptr: i32| {
        match HubHost::list_groups(caller.data_mut()) {
            Ok(json) => write_ok_string(&mut caller, retptr, &json),
            Err(e) => write_denied_at4(&mut caller, retptr, reason_byte(e)),
        }
    })?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "create-group",
        |mut caller: Caller<HostState>, name_ptr: i32, name_len: i32, retptr: i32| {
            let name = read_string(&mut caller, name_ptr, name_len)?;
            match HubHost::create_group(caller.data_mut(), name) {
                Ok(()) => write_ok_unit(&mut caller, retptr),
                Err(e) => write_denied_unit(&mut caller, retptr, reason_byte(e)),
            }
        },
    )?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "set-group",
        |mut caller: Caller<HostState>, id_ptr: i32, id_len: i32, group_ptr: i32, group_len: i32, member: i32, retptr: i32| {
            let id = read_string(&mut caller, id_ptr, id_len)?;
            let group = read_string(&mut caller, group_ptr, group_len)?;
            match HubHost::set_group(caller.data_mut(), id, group, member != 0) {
                Ok(()) => write_ok_unit(&mut caller, retptr),
                Err(e) => write_denied_unit(&mut caller, retptr, reason_byte(e)),
            }
        },
    )?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "remove",
        |mut caller: Caller<HostState>, id_ptr: i32, id_len: i32, retptr: i32| {
            let id = read_string(&mut caller, id_ptr, id_len)?;
            match HubHost::remove(caller.data_mut(), id) {
                Ok(()) => write_ok_unit(&mut caller, retptr),
                Err(e) => write_denied_unit(&mut caller, retptr, reason_byte(e)),
            }
        },
    )?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "search",
        |mut caller: Caller<HostState>, query_ptr: i32, query_len: i32| -> Result<i32, WasmiError> {
            let query = read_string(&mut caller, query_ptr, query_len)?;
            Ok(HubHost::search(caller.data_mut(), query) as i32)
        },
    )?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "install",
        |mut caller: Caller<HostState>, id_ptr: i32, id_len: i32| -> Result<i32, WasmiError> {
            let app = read_string(&mut caller, id_ptr, id_len)?;
            Ok(HubHost::install(caller.data_mut(), app) as i32)
        },
    )?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "app-info",
        |mut caller: Caller<HostState>, id_ptr: i32, id_len: i32, retptr: i32| {
            let id = read_string(&mut caller, id_ptr, id_len)?;
            match HubHost::app_info(caller.data_mut(), id) {
                Ok(json) => write_ok_string(&mut caller, retptr, &json),
                Err(e) => write_denied_at4(&mut caller, retptr, reason_byte(e)),
            }
        },
    )?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "pin",
        |mut caller: Caller<HostState>, id_ptr: i32, id_len: i32, version_ptr: i32, version_len: i32, retptr: i32| {
            let id = read_string(&mut caller, id_ptr, id_len)?;
            let version = read_string(&mut caller, version_ptr, version_len)?;
            match HubHost::pin(caller.data_mut(), id, version) {
                Ok(()) => write_ok_unit(&mut caller, retptr),
                Err(e) => write_denied_unit(&mut caller, retptr, reason_byte(e)),
            }
        },
    )?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "block-publisher",
        |mut caller: Caller<HostState>, key_ptr: i32, key_len: i32, retptr: i32| {
            let key = read_string(&mut caller, key_ptr, key_len)?;
            match HubHost::block_publisher(caller.data_mut(), key) {
                Ok(()) => write_ok_unit(&mut caller, retptr),
                Err(e) => write_denied_unit(&mut caller, retptr, reason_byte(e)),
            }
        },
    )?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "unblock-publisher",
        |mut caller: Caller<HostState>, key_ptr: i32, key_len: i32, retptr: i32| {
            let key = read_string(&mut caller, key_ptr, key_len)?;
            match HubHost::unblock_publisher(caller.data_mut(), key) {
                Ok(()) => write_ok_unit(&mut caller, retptr),
                Err(e) => write_denied_unit(&mut caller, retptr, reason_byte(e)),
            }
        },
    )?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "check-updates",
        |mut caller: Caller<HostState>, id_ptr: i32, id_len: i32| -> Result<i32, WasmiError> {
            let app = read_string(&mut caller, id_ptr, id_len)?;
            Ok(HubHost::check_updates(caller.data_mut(), app) as i32)
        },
    )?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "update",
        |mut caller: Caller<HostState>, id_ptr: i32, id_len: i32| -> Result<i32, WasmiError> {
            let app = read_string(&mut caller, id_ptr, id_len)?;
            Ok(HubHost::update(caller.data_mut(), app) as i32)
        },
    )?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "uninstall",
        |mut caller: Caller<HostState>, id_ptr: i32, id_len: i32, keep_data: i32| -> Result<i32, WasmiError> {
            let app = read_string(&mut caller, id_ptr, id_len)?;
            Ok(HubHost::uninstall(caller.data_mut(), app, keep_data != 0) as i32)
        },
    )?;

    Ok(())
}
