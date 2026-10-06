//! Growable arrays (SPEC.md §4.2). The element kind is one of f64, i32
//! (also booleans and enums) or reference. Only reference arrays are traced.
//!
//! Layout after the header: `len: u32` at 8, `cap: u32` at 12, `data: u32`
//! at 16. `data` is a separate raw buffer.

use crate::gc::{self, T_ARR_F64, T_ARR_I32, T_ARR_REF, load_u32, store_u32};

fn elem_size(p: u32) -> u32 {
    if gc::type_of(p) == T_ARR_F64 { 8 } else { 4 }
}

pub fn new(type_id: u32, cap: u32) -> u32 {
    if !matches!(type_id, T_ARR_F64 | T_ARR_I32 | T_ARR_REF) {
        crate::trap("bad array kind");
    }
    let p = gc::alloc(type_id, 24);
    if cap > 0 {
        let size = if type_id == T_ARR_F64 { 8 } else { 4 };
        unsafe {
            store_u32(p + 12, cap);
            store_u32(p + 16, gc::buffer_alloc((cap * size) as usize));
        }
    }
    p
}

pub fn len(p: u32) -> u32 {
    unsafe { load_u32(p + 8) }
}

fn data(p: u32) -> u32 {
    unsafe { load_u32(p + 16) }
}

pub fn free_data(p: u32) {
    let cap = unsafe { load_u32(p + 12) };
    gc::buffer_free(data(p), (cap * elem_size(p)) as usize);
}

/// Fills the data buffer with the poison pattern in GC stress mode, before
/// it is freed. Test-only (see `gc::set_stress`); normal apps never reach
/// this because `grow` frees old buffers without poisoning them.
pub fn poison_data(p: u32) {
    if gc::stress_enabled() {
        let cap = unsafe { load_u32(p + 12) };
        let d = data(p);
        if d != 0 {
            unsafe { core::ptr::write_bytes(d as *mut u8, 0xAA, (cap * elem_size(p)) as usize) };
        }
    }
}

fn check(p: u32, i: i32) -> u32 {
    if i < 0 || i as u32 >= len(p) {
        crate::trap("array index out of bounds");
    }
    i as u32
}

pub fn get_f64(p: u32, i: i32) -> f64 {
    let i = check(p, i);
    unsafe { *((data(p) + i * 8) as *const f64) }
}

pub fn get_i32(p: u32, i: i32) -> i32 {
    let i = check(p, i);
    unsafe { load_u32(data(p) + i * 4) as i32 }
}

pub fn set_f64(p: u32, i: i32, v: f64) {
    let i = check(p, i);
    unsafe { *((data(p) + i * 8) as *mut f64) = v }
}

pub fn set_i32(p: u32, i: i32, v: i32) {
    let i = check(p, i);
    unsafe { store_u32(data(p) + i * 4, v as u32) }
}

fn grow(p: u32) {
    let (len, cap, size) = (len(p), unsafe { load_u32(p + 12) }, elem_size(p));
    if len < cap {
        return;
    }
    let new_cap = (cap * 2).max(4);
    let buf = gc::buffer_alloc((new_cap * size) as usize);
    unsafe {
        core::ptr::copy_nonoverlapping(data(p) as *const u8, buf as *mut u8, (len * size) as usize);
    }
    free_data(p);
    unsafe {
        store_u32(p + 12, new_cap);
        store_u32(p + 16, buf);
    }
}

pub fn push_f64(p: u32, v: f64) -> u32 {
    grow(p);
    let n = len(p);
    unsafe {
        *((data(p) + n * 8) as *mut f64) = v;
        store_u32(p + 8, n + 1);
    }
    n + 1
}

pub fn push_i32(p: u32, v: i32) -> u32 {
    grow(p);
    let n = len(p);
    unsafe {
        store_u32(data(p) + n * 4, v as u32);
        store_u32(p + 8, n + 1);
    }
    n + 1
}

/// Resolves a (possibly negative) `slice`-style index against `len`, the
/// way JS does: negative counts back from the end, clamped to `[0, len]`.
fn rel_index(i: i32, len: u32) -> u32 {
    if i < 0 { (len as i64 + i as i64).max(0) as u32 } else { (i as u32).min(len) }
}

/// A shallow copy of `[start, end)`, with JS's negative-index semantics
/// (SPEC.md §6.3 keyed lists rely on `slice(0, -1)` etc. behaving this way).
pub fn slice(p: u32, start: i32, end: i32) -> u32 {
    let len = len(p);
    let end = rel_index(end, len);
    let start = rel_index(start, len).min(end);
    let n = end - start;
    let q = new(gc::type_of(p), n);
    let size = elem_size(p);
    unsafe {
        core::ptr::copy_nonoverlapping((data(p) + start * size) as *const u8, data(q) as *mut u8, (n * size) as usize);
        store_u32(q + 8, n);
    }
    q
}

/// Appends all elements of `src` to `dst` (array spread).
pub fn extend(dst: u32, src: u32) {
    if gc::type_of(src) == T_ARR_F64 {
        for i in 0..len(src) {
            push_f64(dst, get_f64(src, i as i32));
        }
    } else {
        for i in 0..len(src) {
            push_i32(dst, get_i32(src, i as i32));
        }
    }
}

pub fn pop_i32(p: u32) -> i32 {
    let n = len(p);
    if n == 0 {
        return 0;
    }
    let v = get_i32(p, (n - 1) as i32);
    unsafe { store_u32(p + 8, n - 1) };
    v
}

pub fn pop_f64(p: u32) -> f64 {
    let n = len(p);
    if n == 0 {
        return f64::NAN;
    }
    let v = get_f64(p, (n - 1) as i32);
    unsafe { store_u32(p + 8, n - 1) };
    v
}

pub fn reverse(p: u32) {
    let n = len(p);
    let size = elem_size(p) as usize;
    let base = data(p) as *mut u8;
    for i in 0..(n / 2) as usize {
        let j = n as usize - 1 - i;
        unsafe { core::ptr::swap_nonoverlapping(base.add(i * size), base.add(j * size), size) };
    }
}
