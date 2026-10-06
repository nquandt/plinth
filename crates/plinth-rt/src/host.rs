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
