//! The hash index of a `Map` or `Set` (core 1.13).
//!
//! A `Map<K, V>` is the compiler's struct `{ keys: K[], values: V[], index:
//! int[] }`; a `Set<T>` is `{ keys: T[], index: int[] }`. The `keys` and
//! `values` arrays keep the entries in insertion order, so iteration is
//! the order of JS. The `index` array is a hash table from a key to its
//! position in `keys`. Only the functions here read or write it.
//!
//! `delete` does not move entries: it marks the entry as a hole. Thus
//! `set`, `get`, `has` and `delete` are O(1) on average. Loops skip the
//! holes with `live`; `keys()`, `values()`, `entries()`, `JSON.stringify`
//! and the `fetch` headers read a copy without the holes (`live_copy`).
//!
//! **Iteration.** A `for…of` or `forEach` loop over a map calls `iter(+1)`
//! before it starts and `iter(-1)` in a `finally` (so also after `break`,
//! `return`, `continue` to an outer loop, or an exception). While the
//! count is above zero, no function here moves an entry: a growing table
//! keeps the holes, and `clear` makes every entry a hole. Thus a loop visits
//! each live entry one time, visits the entries that its body adds, and
//! does not visit the entries that its body deletes, as in JS. When the
//! count goes back to zero, the holes go away if they are many. If the
//! count stays above zero (an `async` function that never continues after
//! an `await` in such a loop), the only effect is that this map keeps its
//! holes: the results stay correct.
//!
//! Layout of `index` (`i32` words): `[0]` the number of entries (with the
//! holes) that the index knows, `[1]` the number of holes, `[2]` the table
//! size `C` (a power of two), `[3]` the number of active loops, then `C`
//! slots (`EMPTY`, `DELETED` or an entry position), then `C / 2` hole flags
//! (1 = hole), one per position. The table is at most half full, so a
//! probe always finds an empty slot.
//!
//! The index is valid only when `[0]` is the length of `keys`. Otherwise
//! (a new map, or a map from `JSON.parse`) the next call builds it again
//! from `keys`. Thus the compiler can make a map with an empty `index`
//! array and fill `keys`/`values` directly.
//!
//! Key kinds come from the kind of the `keys` array: a reference array
//! holds strings (compared by content), an `f64` array numbers (compared
//! with `==`, as before: `NaN` is never found), an `i32` array booleans,
//! `int` values and enums.

use crate::arrays;
use crate::gc::{T_ARR_F64, T_ARR_REF, load_u32, store_u32};
use crate::strings;
use alloc::vec::Vec;

const SYNC: u32 = 0;
const HOLES: u32 = 1;
const CAP: u32 = 2;
const ITER: u32 = 3;
const SLOTS: u32 = 4;
const EMPTY: i32 = -1;
const DELETED: i32 = -2;

#[derive(Clone, Copy)]
enum Key<'a> {
    Str(&'a [u8]),
    F64(f64),
    I32(i32),
}

fn fmix(mut h: u32) -> u32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^ (h >> 16)
}

fn hash(k: Key) -> u32 {
    match k {
        Key::Str(b) => {
            let mut h: u32 = 0x811c_9dc5;
            for &c in b {
                h = (h ^ c as u32).wrapping_mul(0x0100_0193);
            }
            fmix(h)
        }
        // `-0` and `0` are the same key.
        Key::F64(v) => {
            let bits = if v == 0.0 { 0 } else { v.to_bits() };
            fmix((bits as u32) ^ fmix((bits >> 32) as u32))
        }
        Key::I32(v) => fmix(v as u32),
    }
}

fn same(a: Key, b: Key) -> bool {
    match (a, b) {
        (Key::Str(x), Key::Str(y)) => x == y,
        (Key::F64(x), Key::F64(y)) => x == y,
        (Key::I32(x), Key::I32(y)) => x == y,
        _ => false,
    }
}

/// The key at `pos` of `keys`. `pos` must be below the length.
fn key_at<'a>(keys: u32, pos: u32) -> Key<'a> {
    let (data, size) = arrays::raw(keys);
    let at = data + pos * size;
    match crate::gc::type_of(keys) {
        T_ARR_REF => Key::Str(strings::as_str(unsafe { load_u32(at) }).as_bytes()),
        T_ARR_F64 => Key::F64(unsafe { *(at as *const f64) }),
        _ => Key::I32(unsafe { load_u32(at) } as i32),
    }
}

fn word(index: u32, i: u32) -> i32 {
    unsafe { load_u32(arrays::raw(index).0 + i * 4) as i32 }
}

fn set_word(index: u32, i: u32, v: i32) {
    unsafe { store_u32(arrays::raw(index).0 + i * 4, v as u32) }
}

fn has_header(index: u32) -> bool {
    arrays::len(index) > SLOTS
}

fn valid(keys: u32, index: u32) -> bool {
    has_header(index) && word(index, SYNC) as u32 == arrays::len(keys)
}

fn loops(index: u32) -> i32 {
    if has_header(index) { word(index, ITER) } else { 0 }
}

fn flag_at(index: u32, pos: u32) -> u32 {
    SLOTS + word(index, CAP) as u32 + pos
}

/// Moves the live entries of `keys` and `values` (0 for a `Set`) to the
/// front, in order, and drops the holes. `dead[pos]` marks the holes.
fn compact_entries(keys: u32, values: u32, dead: &[bool]) {
    let n = arrays::len(keys);
    let (kd, ks) = arrays::raw(keys);
    let (vd, vs) = if values != 0 { arrays::raw(values) } else { (0, 0) };
    let mut w = 0;
    for pos in 0..n {
        if dead.get(pos as usize).copied().unwrap_or(false) {
            continue;
        }
        if w != pos {
            unsafe {
                core::ptr::copy_nonoverlapping((kd + pos * ks) as *const u8, (kd + w * ks) as *mut u8, ks as usize);
                if values != 0 && pos < arrays::len(values) {
                    core::ptr::copy_nonoverlapping((vd + pos * vs) as *const u8, (vd + w * vs) as *mut u8, vs as usize);
                }
            }
        }
        w += 1;
    }
    arrays::truncate(keys, w);
    if values != 0 {
        arrays::truncate(values, w);
    }
}

/// Builds the index again from `keys`. `known` is `Some(n)` when the index
/// is correct for the first `n` entries: then its holes stay holes, and
/// they are dropped from the arrays only when no loop is active. Keeps the
/// loop count.
fn rebuild(keys: u32, values: u32, index: u32, known: Option<u32>) {
    let active = loops(index);
    let mut dead: Vec<bool> = Vec::new();
    if let Some(known) = known
        && word(index, HOLES) > 0
    {
        dead = (0..known).map(|pos| word(index, flag_at(index, pos)) != 0).collect();
        if active == 0 {
            compact_entries(keys, values, &dead);
            dead.clear();
        }
    }
    let n = arrays::len(keys);
    let mut cap: u32 = 8;
    while cap < n.saturating_mul(4) {
        cap *= 2;
    }
    arrays::reset_i32(index, SLOTS + cap + cap / 2, EMPTY);
    set_word(index, SYNC, n as i32);
    set_word(index, CAP, cap as i32);
    set_word(index, ITER, active);
    let mut holes = 0;
    for pos in 0..cap / 2 {
        let d = dead.get(pos as usize).copied().unwrap_or(false);
        holes += d as i32;
        set_word(index, SLOTS + cap + pos, d as i32);
    }
    set_word(index, HOLES, holes);
    let mask = cap - 1;
    for pos in 0..n {
        if dead.get(pos as usize).copied().unwrap_or(false) {
            continue;
        }
        let k = key_at(keys, pos);
        let mut i = hash(k) & mask;
        loop {
            let s = word(index, SLOTS + i);
            if s == EMPTY {
                set_word(index, SLOTS + i, pos as i32);
                break;
            }
            // A key that is there two times (JSON with the same key two
            // times): the first one is found, as before.
            if same(key_at(keys, s as u32), k) {
                break;
            }
            i = (i + 1) & mask;
        }
    }
}

fn ensure(keys: u32, index: u32) {
    if !valid(keys, index) {
        rebuild(keys, 0, index, None);
    }
}

fn find(keys: u32, index: u32, k: Key) -> i32 {
    ensure(keys, index);
    let mask = word(index, CAP) as u32 - 1;
    let mut i = hash(k) & mask;
    loop {
        let s = word(index, SLOTS + i);
        if s == EMPTY {
            return -1;
        }
        if s >= 0 && same(key_at(keys, s as u32), k) {
            return s;
        }
        i = (i + 1) & mask;
    }
}

pub fn find_str(keys: u32, index: u32, key: u32) -> i32 {
    find(keys, index, Key::Str(strings::as_str(key).as_bytes()))
}

pub fn find_f64(keys: u32, index: u32, key: f64) -> i32 {
    find(keys, index, Key::F64(key))
}

pub fn find_i32(keys: u32, index: u32, key: i32) -> i32 {
    find(keys, index, Key::I32(key))
}

/// Adds the last entry of `keys` to the index. The compiler calls it after
/// it pushes a new key (one that `find` did not find) and its value.
pub fn add(keys: u32, values: u32, index: u32) {
    let n = arrays::len(keys);
    if n == 0 || !has_header(index) || word(index, SYNC) as u32 != n - 1 {
        rebuild(keys, values, index, None);
        return;
    }
    let cap = word(index, CAP) as u32;
    if n > cap / 2 {
        rebuild(keys, values, index, Some(n - 1));
        return;
    }
    let pos = n - 1;
    let mask = cap - 1;
    let mut i = hash(key_at(keys, pos)) & mask;
    while word(index, SLOTS + i) >= 0 {
        i = (i + 1) & mask;
    }
    set_word(index, SLOTS + i, pos as i32);
    set_word(index, SLOTS + cap + pos, 0);
    set_word(index, SYNC, n as i32);
}

/// Makes the entry at `pos` (from `find`) a hole.
pub fn remove(keys: u32, values: u32, index: u32, pos: i32) {
    if pos < 0 || !valid(keys, index) || pos as u32 >= arrays::len(keys) {
        return;
    }
    let pos = pos as u32;
    let cap = word(index, CAP) as u32;
    let mask = cap - 1;
    let mut i = hash(key_at(keys, pos)) & mask;
    loop {
        let s = word(index, SLOTS + i);
        if s == pos as i32 {
            set_word(index, SLOTS + i, DELETED);
            break;
        }
        if s == EMPTY {
            break;
        }
        i = (i + 1) & mask;
    }
    let flag = SLOTS + cap + pos;
    if word(index, flag) != 0 {
        return;
    }
    set_word(index, flag, 1);
    let holes = word(index, HOLES) + 1;
    set_word(index, HOLES, holes);
    // Every entry is a hole and no loop is active: start again with empty
    // arrays.
    if holes as u32 == arrays::len(keys) && loops(index) == 0 {
        clear(keys, values, index);
    }
}

/// `clear()`. With an active loop, every entry becomes a hole (the loop
/// then visits only the entries added after the `clear`); else the arrays
/// become empty.
pub fn clear(keys: u32, values: u32, index: u32) {
    let active = loops(index);
    if active == 0 {
        arrays::truncate(keys, 0);
        if values != 0 {
            arrays::truncate(values, 0);
        }
        rebuild(keys, values, index, None);
        return;
    }
    ensure(keys, index);
    let n = arrays::len(keys);
    let cap = word(index, CAP) as u32;
    for i in 0..cap {
        set_word(index, SLOTS + i, EMPTY);
    }
    for pos in 0..n {
        set_word(index, SLOTS + cap + pos, 1);
    }
    set_word(index, HOLES, n as i32);
}

/// The number of entries (`size`).
pub fn size(keys: u32, index: u32) -> i32 {
    let n = arrays::len(keys) as i32;
    if valid(keys, index) { n - word(index, HOLES) } else { n }
}

/// 1 when the entry at `pos` is not a hole.
pub fn live(keys: u32, index: u32, pos: i32) -> i32 {
    if pos < 0 || !valid(keys, index) || pos as u32 >= arrays::len(keys) {
        return 1;
    }
    (word(index, flag_at(index, pos as u32)) == 0) as i32
}

/// A new array with the elements of `arr` (`keys` or `values`) at the
/// positions that are not holes, in order. The map does not change.
pub fn live_copy(arr: u32, keys: u32, index: u32) -> u32 {
    let n = arrays::len(arr).min(arrays::len(keys));
    let holes = valid(keys, index) && word(index, HOLES) > 0;
    let out = arrays::new(crate::gc::type_of(arr), n);
    let (src, size) = arrays::raw(arr);
    let dst = arrays::raw(out).0;
    let mut w = 0;
    for pos in 0..n {
        if holes && word(index, flag_at(index, pos)) != 0 {
            continue;
        }
        unsafe { core::ptr::copy_nonoverlapping((src + pos * size) as *const u8, (dst + w * size) as *mut u8, size as usize) };
        w += 1;
    }
    arrays::set_len(out, w);
    out
}

/// A loop over the map starts (`delta` 1) or ends (`delta` -1). When the
/// last loop ends and at least half of the entries are holes, the holes go
/// away.
pub fn iter(keys: u32, values: u32, index: u32, delta: i32) {
    ensure(keys, index);
    let count = (word(index, ITER) + delta).max(0);
    set_word(index, ITER, count);
    let holes = word(index, HOLES);
    if count == 0 && holes > 0 && holes as u32 * 2 >= arrays::len(keys) {
        rebuild(keys, values, index, Some(arrays::len(keys)));
    }
}
