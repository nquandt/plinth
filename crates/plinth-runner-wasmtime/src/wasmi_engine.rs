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
//! This step wires `ui`, `dev`, `error`, `time`, `store` and `clipboard`.
//! `dialog`, `net`, `files` and `hub` get shims that trap with a clear
//! message; step 2 replaces them (SPEC.md §9.2).

use crate::bindings::plinth::app::clipboard::Host as ClipboardHost;
use crate::bindings::plinth::app::dev::Host as DevHost;
use crate::bindings::plinth::app::error::{DeniedReason as WitDeniedReason, Host as ErrorHost, HostError};
use crate::bindings::plinth::app::store::Host as StoreHost;
use crate::bindings::plinth::app::time::Host as TimeHost;
use crate::bindings::plinth::app::ui::Host as UiHost;
use crate::hub::HubBackend;
use crate::kv::Kv;
use crate::policy::Policy;
use crate::timers::TimerQueue;
use crate::wasmi_core::extract_core_module;
use crate::{Clipboard, HostState, Limiter, Limits, requests::RequestQueue};
use anyhow::{Context as _, Result};
use std::time::Instant;
use wasmi::{Caller, Config, Engine, Error as WasmiError, Extern, Linker, Memory, Module, Store, TypedFunc};

/// Builds the wasmi engine. No fuel or memory limiter yet (SPEC.md §9.2
/// step 2): `wasmi::Config::consume_fuel` and `Store::limiter` land then.
pub fn new_engine() -> Engine {
    Engine::new(&Config::default())
}

/// One running app instance on wasmi, the wasmi twin of `crate::Guest`'s
/// wasmtime fields.
pub struct WasmiGuest {
    pub(crate) store: Store<HostState>,
    memory: Memory,
    init_fn: TypedFunc<(i32, i32), ()>,
    on_event_fn: TypedFunc<(i32, i32), ()>,
    cabi_realloc: TypedFunc<(i32, i32, i32, i32), i32>,
    /// Set after a trap. A trapped instance is never called again.
    pub(crate) poisoned: bool,
}

/// Extracts the core module, links the host imports, and instantiates it
/// (the wasmi twin of `Runner::load_with_policy_and_hub`).
pub fn load(
    engine: &Engine,
    bytes: &[u8],
    limits: Limits,
    policy: Policy,
    kv: Kv,
    clipboard: Box<dyn Clipboard>,
    hub: Option<Box<dyn HubBackend>>,
) -> Result<WasmiGuest> {
    let core = extract_core_module(bytes).context("extract the linked core module")?;
    let module = Module::new(engine, &core).context("the core module is not valid wasm")?;

    let mut linker = Linker::<HostState>::new(engine);
    register_imports(&mut linker)?;

    let state = HostState {
        commits: Vec::new(),
        logs: Vec::new(),
        errors: Vec::new(),
        limits: Limiter {
            inner: wasmtime::StoreLimitsBuilder::new().memory_size(limits.memory_bytes).instances(4).build(),
            memory: 0,
        },
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
        files: crate::files::Files::unavailable(),
    };
    let mut store = Store::new(engine, state);
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

    Ok(WasmiGuest { store, memory, init_fn, on_event_fn, cabi_realloc, poisoned: false })
}

/// Calls `init` or `on-event` with `bytes` as its `list<u8>` argument: the
/// same buffer passing wasmtime/bindgen does, by hand (`cabi_realloc` in
/// the guest, then a plain `(ptr, len)` call).
pub fn call(g: &mut WasmiGuest, export: &str, bytes: &[u8]) -> Result<()> {
    let ptr = g.cabi_realloc.call(&mut g.store, (0, 0, 1, bytes.len().max(1) as i32)).map_err(anyhow::Error::from)?;
    g.memory.write(&mut g.store, ptr as usize, bytes).map_err(|e| anyhow::anyhow!("write the guest buffer: {e:#}"))?;
    let f = if export == "init" { &g.init_fn } else { &g.on_event_fn };
    f.call(&mut g.store, (ptr, bytes.len() as i32)).map_err(anyhow::Error::from)
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

/// A function in `dialog`/`net`/`files`/`hub` (step 2, SPEC.md §9.2): not
/// wired on wasmi yet. Always traps.
fn not_yet(iface: &str, name: &str) -> WasmiError {
    WasmiError::new(format!("plinth:app/{iface} {name}: not yet on wasmi (SPEC.md §9.2 step 2)"))
}

/// Registers every host import this step wires (`ui`, `dev`, `error`,
/// `time`, `store`, `clipboard`), plus a trap shim for every function of
/// `dialog`, `net`, `files` and `hub` (step 2). A shim is only ever called
/// if the app actually imports it: an app with no `plinth:dialog` import,
/// for example, never reaches `not_yet`.
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

    register_not_yet_shims(linker)?;
    Ok(())
}

/// Step-2 interfaces (SPEC.md §9.2): a shim for every function, with the
/// same core-ABI signature `web/plinth-web.js` documents, that always
/// traps.
fn register_not_yet_shims(linker: &mut Linker<HostState>) -> Result<()> {
    macro_rules! trap2 {
        ($iface:literal, $name:literal) => {
            linker.func_wrap(concat!("plinth:app/", $iface, "@1.0.0"), $name, |_: Caller<HostState>, _: i32, _: i32| -> Result<i32, WasmiError> {
                Err(not_yet($iface, $name))
            })?;
        };
    }
    trap2!("dialog", "alert");
    trap2!("dialog", "confirm");
    trap2!("dialog", "prompt");

    linker.func_wrap(
        "plinth:app/net@1.0.0",
        "fetch",
        |_: Caller<HostState>, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| -> Result<i32, WasmiError> {
            Err(not_yet("net", "fetch"))
        },
    )?;

    trap2!("files", "read");
    trap2!("files", "list-dir");
    trap2!("files", "stat");
    trap2!("files", "remove");
    linker.func_wrap(
        "plinth:app/files@1.0.0",
        "write",
        |_: Caller<HostState>, _: i32, _: i32, _: i32, _: i32| -> Result<i32, WasmiError> { Err(not_yet("files", "write")) },
    )?;

    linker.func_wrap("plinth:app/hub@1.0.0", "list-apps", |_: Caller<HostState>, _: i32| -> Result<(), WasmiError> {
        Err(not_yet("hub", "list-apps"))
    })?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "launch",
        |_: Caller<HostState>, _: i32, _: i32, _: i32| -> Result<(), WasmiError> { Err(not_yet("hub", "launch")) },
    )?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "set-grant",
        |_: Caller<HostState>, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| -> Result<(), WasmiError> {
            Err(not_yet("hub", "set-grant"))
        },
    )?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "block",
        |_: Caller<HostState>, _: i32, _: i32, _: i32| -> Result<(), WasmiError> { Err(not_yet("hub", "block")) },
    )?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "unblock",
        |_: Caller<HostState>, _: i32, _: i32, _: i32| -> Result<(), WasmiError> { Err(not_yet("hub", "unblock")) },
    )?;
    linker.func_wrap("plinth:app/hub@1.0.0", "list-groups", |_: Caller<HostState>, _: i32| -> Result<(), WasmiError> {
        Err(not_yet("hub", "list-groups"))
    })?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "create-group",
        |_: Caller<HostState>, _: i32, _: i32, _: i32| -> Result<(), WasmiError> { Err(not_yet("hub", "create-group")) },
    )?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "set-group",
        |_: Caller<HostState>, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| -> Result<(), WasmiError> {
            Err(not_yet("hub", "set-group"))
        },
    )?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "remove",
        |_: Caller<HostState>, _: i32, _: i32, _: i32| -> Result<(), WasmiError> { Err(not_yet("hub", "remove")) },
    )?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "search",
        |_: Caller<HostState>, _: i32, _: i32| -> Result<i32, WasmiError> { Err(not_yet("hub", "search")) },
    )?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "install",
        |_: Caller<HostState>, _: i32, _: i32| -> Result<i32, WasmiError> { Err(not_yet("hub", "install")) },
    )?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "app-info",
        |_: Caller<HostState>, _: i32, _: i32, _: i32| -> Result<(), WasmiError> { Err(not_yet("hub", "app-info")) },
    )?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "pin",
        |_: Caller<HostState>, _: i32, _: i32, _: i32, _: i32, _: i32| -> Result<(), WasmiError> { Err(not_yet("hub", "pin")) },
    )?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "block-publisher",
        |_: Caller<HostState>, _: i32, _: i32, _: i32| -> Result<(), WasmiError> { Err(not_yet("hub", "block-publisher")) },
    )?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "unblock-publisher",
        |_: Caller<HostState>, _: i32, _: i32, _: i32| -> Result<(), WasmiError> { Err(not_yet("hub", "unblock-publisher")) },
    )?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "check-updates",
        |_: Caller<HostState>, _: i32, _: i32| -> Result<i32, WasmiError> { Err(not_yet("hub", "check-updates")) },
    )?;
    linker.func_wrap(
        "plinth:app/hub@1.0.0",
        "update",
        |_: Caller<HostState>, _: i32, _: i32| -> Result<i32, WasmiError> { Err(not_yet("hub", "update")) },
    )?;

    Ok(())
}
