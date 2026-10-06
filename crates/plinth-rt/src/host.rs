//! Guest-side wrappers for the host APIs (`plinth:time`, `plinth:store`,
//! `plinth:clipboard`, SPEC.md §8.5). Each wrapper adapts between the
//! `__plinth_rt_*` ABI (i32/f64 values, heap string pointers) and the
//! `wit-bindgen`-generated `bindings::plinth::app::*` calls.
//!
//! A denied call (`host-error.denied`) never traps (SPEC.md §8.5): it is
//! reported to the app as a thrown `PlinthError` by the compiler's
//! lowering of these calls (see `plinth-compiler`'s `lower.rs`), using
//! `__plinth_rt_throw`.

use crate::global::Global;
use crate::{Callable, Val, invoke, strings};
use alloc::string::String;
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

#[cfg(target_arch = "wasm32")]
fn host_denied_trap(what: &str) -> ! {
    let mut line = String::new();
    line.push_str(what);
    line.push_str(": denied");
    crate::trap(&line);
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
/// id for `clearTimeout`/`clearInterval`.
pub fn set_timer(callable: Callable, ms: i32, repeat: bool) -> i32 {
    #[cfg(target_arch = "wasm32")]
    {
        match host::time::set_timer(ms.max(0) as u32, repeat) {
            Ok(id) => {
                TIMERS.with(|t| t.push(Timer { id, callable, one_shot: !repeat }));
                id as i32
            }
            Err(_) => host_denied_trap("setTimeout"),
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (callable, ms, repeat);
        0
    }
}

/// `clearTimeout`/`clearInterval`.
pub fn clear_timer(id: i32) {
    #[cfg(target_arch = "wasm32")]
    {
        host::time::cancel_timer(id as u32);
        TIMERS.with(|t| t.retain(|x| x.id != id as u32));
    }
}

/// `store.kv.get(key)`: returns a string pointer, or `0` (null) when the
/// key is not set.
pub fn kv_get(key: i32) -> i32 {
    #[cfg(target_arch = "wasm32")]
    {
        let key = strings::as_str(key as u32);
        match host::store::kv_get(key) {
            Ok(Some(v)) => strings::from_str(&v) as i32,
            Ok(None) => 0,
            Err(_) => host_denied_trap("store.kv.get"),
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = key;
        0
    }
}

/// `store.kv.set(key, value)`.
pub fn kv_set(key: i32, value: i32) {
    #[cfg(target_arch = "wasm32")]
    {
        let key = strings::as_str(key as u32);
        let value = strings::as_str(value as u32);
        if host::store::kv_set(key, value).is_err() {
            host_denied_trap("store.kv.set");
        }
    }
}

/// `store.kv.remove(key)`.
pub fn kv_delete(key: i32) {
    #[cfg(target_arch = "wasm32")]
    {
        let key = strings::as_str(key as u32);
        if host::store::kv_delete(key).is_err() {
            host_denied_trap("store.kv.remove");
        }
    }
}

/// `store.kv.keys()`: returns a guest `Array<string>` pointer. Built with
/// the array helpers in `arrays.rs` so the GC can trace it.
pub fn kv_keys() -> i32 {
    #[cfg(target_arch = "wasm32")]
    {
        match host::store::kv_keys() {
            Ok(keys) => {
                let arr = crate::arrays::new(crate::gc::T_ARR_REF, keys.len() as u32);
                for k in keys {
                    let s = strings::from_str(&k);
                    crate::arrays::push_i32(arr, s as i32);
                }
                arr as i32
            }
            Err(_) => host_denied_trap("store.kv.keys"),
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    0
}

/// `clipboard.writeText(text)`.
pub fn clipboard_write_text(text: i32) {
    #[cfg(target_arch = "wasm32")]
    {
        let text = strings::as_str(text as u32);
        if host::clipboard::write_text(text).is_err() {
            host_denied_trap("clipboard.writeText");
        }
    }
}

/// `clipboard.readText()`: returns a string pointer, or `0` (null) when
/// the clipboard holds no text.
pub fn clipboard_read_text() -> i32 {
    #[cfg(target_arch = "wasm32")]
    {
        match host::clipboard::read_text() {
            Ok(Some(v)) => strings::from_str(&v) as i32,
            Ok(None) => 0,
            Err(_) => host_denied_trap("clipboard.readText"),
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    0
}
