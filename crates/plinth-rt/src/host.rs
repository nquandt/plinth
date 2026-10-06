//! Guest-side wrappers for the host APIs (`plinth:time`, `plinth:store`,
//! `plinth:clipboard`, SPEC.md §8.5). Each wrapper adapts between the
//! `__plinth_rt_*` ABI (i32/f64 values, heap string pointers) and the
//! `wit-bindgen`-generated `bindings::plinth::app::*` calls.
//!
//! SPEC.md §8.5: a denied host call never traps. `kv.get`/`clipboard.
//! readText` return `null`, `kv.set`/`kv.remove`/`clipboard.writeText` do
//! nothing, and `kv.keys` returns `[]`. The denial reason is recorded and
//! made visible to the app through `kv.lastError()` /
//! `clipboard.lastError()`, which return one of `"denied:undeclared"`,
//! `"denied:refused"`, `"denied:unsupported"`, or `null` if the last call
//! of that module was not denied; a successful call of that module clears
//! it back to `null`. `time` needs no capability at all (SPEC.md §11), so
//! `setTimeout`/`setInterval` cannot be denied.

// The non-wasm32 (host test) build of each wrapper below ignores its
// arguments; only wasm32 apps ever call them for real.
#![cfg_attr(not(target_arch = "wasm32"), allow(unused_variables))]

use crate::global::Global;
use crate::{Callable, Val, invoke};
#[cfg(target_arch = "wasm32")]
use crate::strings;
use alloc::vec::Vec;

#[cfg(target_arch = "wasm32")]
use crate::bindings::plinth::app as host;

/// A pending timer: the id the host gave us, and the callback to invoke.
struct Timer {
    id: u32,
    callable: Callable,
    /// So we can tell the caller (`clear_timer`) which guest-side token
    /// maps to which host id; here they are the same value, but kept as
    /// a separate field to make that assumption visible.
    one_shot: bool,
}

static TIMERS: Global<Vec<Timer>> = Global::new(Vec::new());

/// Gives every heap reference that a pending timer holds. Without this, a
/// timer's closure environment looks unreachable to the collector between
/// the timer firing and `setTimer`/`clearTimer`, and the GC frees it: the
/// next firing then calls through a dangling function-table index (SPEC.md
/// §5.4; found by the GC stress-mode `timer` test, `gc_stress.rs`).
pub fn trace_roots(f: &mut dyn FnMut(u32)) {
    TIMERS.with(|t| {
        for timer in t.iter() {
            f(timer.callable.env);
        }
    });
    // A pending request's callback must survive a GC between the host call
    // and its `completion` event, exactly like a pending timer above.
    REQUESTS.with(|r| {
        for req in r.iter() {
            f(req.callable.env);
        }
    });
}

/// Dispatches a `timer` event (SPEC.md §8.4) to its registered callback,
/// mirroring `ui::dispatch`'s handler lookup. One-shot timers are removed
/// after firing (the host already drops its own copy); repeating timers
/// stay registered.
pub fn dispatch_timer(timer: u32) {
    let (callable, one_shot) =
        match TIMERS.with(|t| t.iter().position(|x| x.id == timer).map(|i| (t[i].callable, t[i].one_shot))) {
            Some(x) => x,
            None => return, // Canceled or unknown; not an error.
        };
    if one_shot {
        TIMERS.with(|t| t.retain(|x| x.id != timer));
    }
    crate::reactive::untracked(|| invoke(callable, Val::None));
}

/// A pending async host call: the request id the host gave us, and the
/// callback to invoke when its `completion` event (SPEC.md §8.4) arrives.
struct Request {
    id: u32,
    callable: Callable,
}

static REQUESTS: Global<Vec<Request>> = Global::new(Vec::new());

/// Registers `callable` under the request id a host API call returned, so
/// `dispatch_completion` can find it when the completion arrives.
pub fn register_request(id: u32, callable: Callable) {
    REQUESTS.with(|r| r.push(Request { id, callable }));
}

/// Dispatches a `completion` event (SPEC.md §8.4) to its registered
/// callback with the decoded result, then forgets the request. An unknown
/// (already-answered, or never registered) request id is ignored, not an
/// error.
pub fn dispatch_completion(request: u32, result: &plinth_protocol::Value) {
    // `plinth:net`'s result is a 4-element list (SPEC.md §8.4): stash it so
    // `net_result_*` can read it, since a single `Val` word cannot carry
    // four fields. Completions are dispatched one at a time (the guest is
    // single-threaded, SPEC.md §4.5), so one global slot is enough.
    if let plinth_protocol::Value::List(items) = result {
        LAST_LIST.with(|l| *l = items.clone());
    }
    let callable = REQUESTS.with(|r| {
        let i = r.iter().position(|x| x.id == request)?;
        Some(r.remove(i).callable)
    });
    if let Some(callable) = callable {
        crate::reactive::untracked(|| invoke(callable, crate::ui::value_to_val(result)));
    }
}

/// The last `plinth:net` completion's result list, read by `net_result_*`
/// (SPEC.md §8.4). Set by `dispatch_completion` just before it invokes the
/// net-fetch wrapper closure the compiler generates (`check/stdlib.rs`).
static LAST_LIST: Global<Vec<plinth_protocol::Value>> = Global::new(Vec::new());

/// The last denial reason for `plinth:store`'s `kv.*` calls, as the string
/// `kv.lastError()` returns, or `None` if the last call was not denied.
/// Cleared on the next successful `kv` call.
static KV_LAST_ERROR: Global<Option<&'static str>> = Global::new(None);
/// Same as `KV_LAST_ERROR`, for `plinth:clipboard`.
static CLIPBOARD_LAST_ERROR: Global<Option<&'static str>> = Global::new(None);

#[cfg(target_arch = "wasm32")]
fn denied_reason(e: host::error::HostError) -> &'static str {
    match e {
        host::error::HostError::Denied(host::error::DeniedReason::Undeclared) => "denied:undeclared",
        host::error::HostError::Denied(host::error::DeniedReason::Refused) => "denied:refused",
        host::error::HostError::Denied(host::error::DeniedReason::Unsupported) => "denied:unsupported",
    }
}

/// `kv.lastError()`: the reason the last `kv` call was denied, or `null`.
pub fn kv_last_error() -> i32 {
    #[cfg(target_arch = "wasm32")]
    return KV_LAST_ERROR.with(|e| match e {
        Some(s) => strings::from_str(s) as i32,
        None => 0,
    });
    #[cfg(not(target_arch = "wasm32"))]
    0
}

/// `clipboard.lastError()`: the reason the last clipboard call was denied,
/// or `null`.
pub fn clipboard_last_error() -> i32 {
    #[cfg(target_arch = "wasm32")]
    return CLIPBOARD_LAST_ERROR.with(|e| match e {
        Some(s) => strings::from_str(s) as i32,
        None => 0,
    });
    #[cfg(not(target_arch = "wasm32"))]
    0
}

/// `time.now()`: milliseconds since the Unix epoch.
pub fn now() -> f64 {
    #[cfg(target_arch = "wasm32")]
    return host::time::now() as f64;
    #[cfg(not(target_arch = "wasm32"))]
    0.0
}

/// `time.monotonic-now()`: monotonic milliseconds.
pub fn monotonic_now() -> f64 {
    #[cfg(target_arch = "wasm32")]
    return host::time::monotonic_now() as f64;
    #[cfg(not(target_arch = "wasm32"))]
    0.0
}

/// `time.timezone-offset(ms)`: minutes east of UTC at that instant
/// (docs/GAPS.md gap #5). Used by `dateParts`/`makeDate`/`formatDate`/
/// `parseDate` for local time.
pub fn timezone_offset(ms: i64) -> i32 {
    #[cfg(target_arch = "wasm32")]
    return host::time::timezone_offset(ms);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = ms;
        0
    }
}

/// `setTimeout`/`setInterval`: schedules `callable` and returns a timer
/// id (as a `number`) for `clearTimeout`/`clearInterval`.
pub fn set_timer(callable: Callable, ms: f64, repeat: bool) -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        // `time` needs no capability (SPEC.md §11): it can never be denied,
        // so there is no error path to handle here.
        match host::time::set_timer(ms.max(0.0) as u32, repeat) {
            Ok(id) => {
                TIMERS.with(|t| t.push(Timer { id, callable, one_shot: !repeat }));
                id as f64
            }
            Err(_) => 0.0,
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (callable, ms, repeat);
        0.0
    }
}

/// `clearTimeout`/`clearInterval`.
pub fn clear_timer(id: f64) {
    #[cfg(target_arch = "wasm32")]
    {
        let id = id as u32;
        host::time::cancel_timer(id);
        TIMERS.with(|t| t.retain(|x| x.id != id));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = id;
    }
}

/// `store.kv.get(key)`: returns a string pointer, or `0` (null) when the
/// key is not set, or the call was denied.
pub fn kv_get(key: i32) -> i32 {
    #[cfg(target_arch = "wasm32")]
    {
        let key = strings::as_str(key as u32);
        match host::store::kv_get(key) {
            Ok(Some(v)) => {
                KV_LAST_ERROR.with(|e| *e = None);
                strings::from_str(&v) as i32
            }
            Ok(None) => {
                KV_LAST_ERROR.with(|e| *e = None);
                0
            }
            Err(e) => {
                KV_LAST_ERROR.with(|slot| *slot = Some(denied_reason(e)));
                0
            }
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = key;
        0
    }
}

/// `store.kv.set(key, value)`. A no-op when denied.
pub fn kv_set(key: i32, value: i32) {
    #[cfg(target_arch = "wasm32")]
    {
        let key = strings::as_str(key as u32);
        let value = strings::as_str(value as u32);
        match host::store::kv_set(key, value) {
            Ok(_) => KV_LAST_ERROR.with(|e| *e = None),
            Err(e) => KV_LAST_ERROR.with(|slot| *slot = Some(denied_reason(e))),
        }
    }
}

/// `store.kv.remove(key)`. A no-op when denied.
pub fn kv_delete(key: i32) {
    #[cfg(target_arch = "wasm32")]
    {
        let key = strings::as_str(key as u32);
        match host::store::kv_delete(key) {
            Ok(_) => KV_LAST_ERROR.with(|e| *e = None),
            Err(e) => KV_LAST_ERROR.with(|slot| *slot = Some(denied_reason(e))),
        }
    }
}

/// `store.kv.keys()`: returns a guest `Array<string>` pointer, empty when
/// denied. Built with the array helpers in `arrays.rs` so the GC can trace
/// it.
pub fn kv_keys() -> i32 {
    #[cfg(target_arch = "wasm32")]
    {
        match host::store::kv_keys() {
            Ok(keys) => {
                KV_LAST_ERROR.with(|e| *e = None);
                let arr = crate::arrays::new(crate::gc::T_ARR_REF, keys.len() as u32);
                for k in keys {
                    let s = strings::from_str(&k);
                    crate::arrays::push_i32(arr, s as i32);
                }
                arr as i32
            }
            Err(e) => {
                KV_LAST_ERROR.with(|slot| *slot = Some(denied_reason(e)));
                crate::arrays::new(crate::gc::T_ARR_REF, 0) as i32
            }
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    0
}

/// `clipboard.writeText(text)`. A no-op when denied.
pub fn clipboard_write_text(text: i32) {
    #[cfg(target_arch = "wasm32")]
    {
        let text = strings::as_str(text as u32);
        match host::clipboard::write_text(text) {
            Ok(_) => CLIPBOARD_LAST_ERROR.with(|e| *e = None),
            Err(e) => CLIPBOARD_LAST_ERROR.with(|slot| *slot = Some(denied_reason(e))),
        }
    }
}

/// `clipboard.readText()`: returns a string pointer, or `0` (null) when
/// the clipboard holds no text, or the call was denied.
pub fn clipboard_read_text() -> i32 {
    #[cfg(target_arch = "wasm32")]
    {
        match host::clipboard::read_text() {
            Ok(Some(v)) => {
                CLIPBOARD_LAST_ERROR.with(|e| *e = None);
                strings::from_str(&v) as i32
            }
            Ok(None) => {
                CLIPBOARD_LAST_ERROR.with(|e| *e = None);
                0
            }
            Err(e) => {
                CLIPBOARD_LAST_ERROR.with(|slot| *slot = Some(denied_reason(e)));
                0
            }
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    0
}

/// `dialog.alert(message, done)`: shows a host-owned modal with an OK
/// button (SPEC.md §8.5). `done` is called with no argument once the user
/// dismisses it. No capability is needed: it is UI, not data access.
pub fn dialog_alert(callable: Callable, message: i32) {
    #[cfg(target_arch = "wasm32")]
    {
        let message = strings::as_str(message as u32);
        let id = host::dialog::alert(message);
        register_request(id, callable);
    }
}

/// `dialog.confirm(message, done)`: shows a host-owned modal with OK and
/// Cancel. `done` is called with a `boolean` (`true` for OK).
pub fn dialog_confirm(callable: Callable, message: i32) {
    #[cfg(target_arch = "wasm32")]
    {
        let message = strings::as_str(message as u32);
        let id = host::dialog::confirm(message);
        register_request(id, callable);
    }
}

/// `dialog.prompt(message, done)`: shows a host-owned modal with a text
/// field. `done` is called with the entered text, or `null` if the user
/// canceled.
pub fn dialog_prompt(callable: Callable, message: i32) {
    #[cfg(target_arch = "wasm32")]
    {
        let message = strings::as_str(message as u32);
        let id = host::dialog::prompt(message);
        register_request(id, callable);
    }
}

/// Reads the `(keys, values)` string arrays backing a `Map<string, string>`
/// (`check/mod.rs`'s `map_struct`: a 2-field struct of arrays, always in
/// that order, so these offsets are stable for any `Map<string, string>`
/// the compiler builds). Used for `net.fetch`'s `headers`.
#[cfg(target_arch = "wasm32")]
fn map_to_pairs(map: u32) -> alloc::vec::Vec<(alloc::string::String, alloc::string::String)> {
    use alloc::string::ToString;
    if map == 0 {
        return alloc::vec::Vec::new();
    }
    let keys = unsafe { crate::gc::load_u32(map + 8) };
    let vals = unsafe { crate::gc::load_u32(map + 12) };
    let n = crate::arrays::len(keys);
    let mut out = alloc::vec::Vec::with_capacity(n as usize);
    for i in 0..n as i32 {
        let k = strings::as_str(crate::arrays::get_i32(keys, i) as u32).to_string();
        let v = strings::as_str(crate::arrays::get_i32(vals, i) as u32).to_string();
        out.push((k, v));
    }
    out
}

/// `net.fetch(url, method, headers, body, done)` (SPEC.md §8.5, §11):
/// `headers` is a `Map<string, string>` pointer, or `0` for none. `body`
/// is a nullable string pointer. The compiler's wrapper closure (built in
/// `check/stdlib.rs`) decodes the completion's result list through
/// `net_result_*` and calls the app's `done` with the decoded `Response`.
pub fn net_fetch(callable: Callable, url: i32, method: i32, headers: i32, body: i32) {
    #[cfg(target_arch = "wasm32")]
    {
        let url = strings::as_str(url as u32);
        let method = strings::as_str(method as u32);
        let headers = map_to_pairs(headers as u32);
        let body = if body == 0 { None } else { Some(strings::as_str(body as u32)) };
        let id = host::net::fetch(url, method, &headers, body);
        register_request(id, callable);
    }
}

/// The last `net.fetch` completion's `ok` field (SPEC.md §8.4).
pub fn net_result_ok() -> i32 {
    LAST_LIST.with(|l| matches!(l.first(), Some(plinth_protocol::Value::Bool(true))) as i32)
}

/// The last `net.fetch` completion's `status` field, as a `number`. Native
/// hosts send `Value::Int`; the web host's JS encoder has no `int` tag for
/// a plain number literal, so it sends `Value::Number` instead (both are
/// accepted here).
pub fn net_result_status() -> f64 {
    LAST_LIST.with(|l| match l.get(1) {
        Some(plinth_protocol::Value::Int(i)) => *i as f64,
        Some(plinth_protocol::Value::Number(n)) => *n,
        _ => 0.0,
    })
}

/// The last `net.fetch` completion's `text` field.
pub fn net_result_text() -> i32 {
    LAST_LIST.with(|l| match l.get(2) {
        #[cfg(target_arch = "wasm32")]
        Some(plinth_protocol::Value::Str(s)) => strings::from_str(s) as i32,
        _ => 0,
    })
}

/// The last `net.fetch` completion's `error` field, or `null`.
pub fn net_result_error() -> i32 {
    LAST_LIST.with(|l| match l.get(3) {
        #[cfg(target_arch = "wasm32")]
        Some(plinth_protocol::Value::Str(s)) => strings::from_str(s) as i32,
        _ => 0,
    })
}
