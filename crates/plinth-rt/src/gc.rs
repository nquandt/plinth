//! The GC heap (SPEC.md §5.4): a non-moving mark-sweep collector.
//!
//! The collector runs only when the guest is idle, that is, after `init` or
//! `on-event` has finished its work. Thus there are no stack roots. The roots
//! are the permanent roots (module environments and string literals) and the
//! references that the runtime keeps (signals, callables, list rows).
//!
//! Object layout. Every object starts with an 8-byte header:
//!
//! | offset | field                          |
//! |--------|--------------------------------|
//! | 0      | `type_id: u32`                 |
//! | 4      | `flags: u32` (bit 0 = mark)    |
//!
//! Built-in types follow the header with:
//!
//! - string: `len: u32` (bytes), then the UTF-8 bytes
//! - arrays: `len: u32`, `cap: u32`, `data: u32` (a separate buffer)
//! - closure: `fn: u32` (table index), `env: u32` (a reference or 0)
//! - box f64: 8 bytes of padding to offset 8, then `value: f64`
//!
//! Compiler-defined types (structs and environments) are registered at start
//! with their size and the offsets of their reference fields.

use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::cell::RefCell;

pub const T_STRING: u32 = 0;
pub const T_ARR_F64: u32 = 1;
pub const T_ARR_I32: u32 = 2;
pub const T_ARR_REF: u32 = 3;
pub const T_CLOSURE: u32 = 4;
pub const T_BOX_F64: u32 = 5;
pub const FIRST_USER_TYPE: u32 = 16;

pub const HEADER: u32 = 8;
const MARK: u32 = 1;
const ALIGN: usize = 8;
/// Collect after this many bytes were allocated since the last collection.
const GC_THRESHOLD: usize = 1 << 20;

struct TypeDesc {
    size: u32,
    ref_offsets: Vec<u32>,
}

#[derive(Default)]
struct Heap {
    objects: Vec<u32>,
    types: Vec<TypeDesc>,
    roots: Vec<u32>,
    allocated_since_gc: usize,
}

thread_local! {
    static HEAP: RefCell<Heap> = RefCell::new(Heap::default());
}

#[inline]
pub unsafe fn load_u32(addr: u32) -> u32 {
    unsafe { *(addr as *const u32) }
}

#[inline]
pub unsafe fn store_u32(addr: u32, v: u32) {
    unsafe { *(addr as *mut u32) = v }
}

fn raw_alloc(size: usize) -> u32 {
    let layout = Layout::from_size_align(size.max(1), ALIGN).expect("layout");
    let p = unsafe { alloc_zeroed(layout) };
    if p.is_null() {
        crate::trap("out of memory");
    }
    p as u32
}

fn raw_free(ptr: u32, size: usize) {
    let layout = Layout::from_size_align(size.max(1), ALIGN).expect("layout");
    unsafe { dealloc(ptr as *mut u8, layout) }
}

/// Allocates a raw buffer that the GC does not manage (array data).
pub fn buffer_alloc(size: usize) -> u32 {
    raw_alloc(size)
}

pub fn buffer_free(ptr: u32, size: usize) {
    if ptr != 0 {
        raw_free(ptr, size)
    }
}

/// Allocates a zeroed object of `size` bytes (the header included).
pub fn alloc(type_id: u32, size: u32) -> u32 {
    let p = raw_alloc(size as usize);
    unsafe { store_u32(p, type_id) };
    HEAP.with_borrow_mut(|h| {
        h.objects.push(p);
        h.allocated_since_gc += size as usize;
    });
    p
}

/// Allocates an object of a registered compiler-defined type.
pub fn alloc_user(type_id: u32) -> u32 {
    let size = HEAP.with_borrow(|h| {
        h.types
            .get((type_id - FIRST_USER_TYPE) as usize)
            .map(|t| t.size)
            .unwrap_or_else(|| crate::trap("unknown type id"))
    });
    alloc(type_id, size)
}

/// Registers compiler-defined types. The table is a sequence of u32 values:
/// `count`, then for each type `size, ref_count, ref_offsets...`.
pub fn register_types(table: &[u32]) {
    let mut it = table.iter().copied();
    let count = it.next().unwrap_or(0);
    HEAP.with_borrow_mut(|h| {
        for _ in 0..count {
            let size = it.next().unwrap_or(HEADER);
            let n = it.next().unwrap_or(0);
            let ref_offsets = (0..n).map(|_| it.next().unwrap_or(0)).collect();
            h.types.push(TypeDesc { size, ref_offsets });
        }
    });
}

pub fn add_root(ptr: u32) {
    if ptr != 0 {
        HEAP.with_borrow_mut(|h| h.roots.push(ptr));
    }
}

pub fn type_of(ptr: u32) -> u32 {
    unsafe { load_u32(ptr) }
}

fn object_size(h: &Heap, ptr: u32) -> usize {
    match type_of(ptr) {
        T_STRING => 12 + unsafe { load_u32(ptr + 8) } as usize,
        T_ARR_F64 | T_ARR_I32 | T_ARR_REF => 24,
        T_CLOSURE => 16,
        T_BOX_F64 => 16,
        t => h.types[(t - FIRST_USER_TYPE) as usize].size as usize,
    }
}

pub fn should_collect() -> bool {
    HEAP.with_borrow(|h| h.allocated_since_gc >= GC_THRESHOLD)
}

/// Runs one full collection. `runtime_roots` gives every reference that the
/// runtime holds outside the heap.
pub fn collect(runtime_roots: impl FnOnce(&mut dyn FnMut(u32))) {
    let mut work: Vec<u32> = HEAP.with_borrow(|h| h.roots.clone());
    runtime_roots(&mut |p| {
        if p != 0 {
            work.push(p)
        }
    });
    HEAP.with_borrow_mut(|h| {
        while let Some(p) = work.pop() {
            let flags = unsafe { load_u32(p + 4) };
            if flags & MARK != 0 {
                continue;
            }
            unsafe { store_u32(p + 4, flags | MARK) };
            match type_of(p) {
                T_STRING | T_ARR_F64 | T_ARR_I32 | T_BOX_F64 => {}
                T_ARR_REF => {
                    let (len, data) = unsafe { (load_u32(p + 8), load_u32(p + 16)) };
                    for i in 0..len {
                        let child = unsafe { load_u32(data + i * 4) };
                        if child != 0 {
                            work.push(child);
                        }
                    }
                }
                T_CLOSURE => {
                    let env = unsafe { load_u32(p + 12) };
                    if env != 0 {
                        work.push(env);
                    }
                }
                t => {
                    for &off in &h.types[(t - FIRST_USER_TYPE) as usize].ref_offsets {
                        let child = unsafe { load_u32(p + off) };
                        if child != 0 {
                            work.push(child);
                        }
                    }
                }
            }
        }
        let mut objects = std::mem::take(&mut h.objects);
        objects.retain(|&p| {
            let flags = unsafe { load_u32(p + 4) };
            if flags & MARK != 0 {
                unsafe { store_u32(p + 4, flags & !MARK) };
                true
            } else {
                if matches!(type_of(p), T_ARR_F64 | T_ARR_I32 | T_ARR_REF) {
                    crate::arrays::free_data(p);
                }
                let size = object_size(h, p);
                raw_free(p, size);
                false
            }
        });
        h.objects = objects;
        h.allocated_since_gc = 0;
    });
}

/// The number of live objects. Tests use it.
pub fn live_objects() -> usize {
    HEAP.with_borrow(|h| h.objects.len())
}
