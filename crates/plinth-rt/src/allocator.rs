//! The memory allocator and the panic handler for wasm32 (SPEC.md §5.4).
//!
//! Small blocks come from size-class free lists; large blocks from a
//! first-fit free list. New memory comes from `memory.grow`. The allocator
//! never returns memory to the host.

use core::alloc::{GlobalAlloc, Layout};
use core::arch::wasm32;
use core::cell::UnsafeCell;

const CLASSES: [usize; 14] = [16, 32, 48, 64, 96, 128, 192, 256, 384, 512, 768, 1024, 1536, 2048];
const LARGE_UNIT: usize = 1024;
const PAGE: usize = 65536;

struct State {
    free: [usize; CLASSES.len()],
    /// Free large blocks: each starts with `[size, next]`.
    large: usize,
    bump: usize,
    end: usize,
}

struct Allocator(UnsafeCell<State>);

// SAFETY: the guest is single-threaded.
unsafe impl Sync for Allocator {}

#[global_allocator]
static ALLOCATOR: Allocator = Allocator(UnsafeCell::new(State { free: [0; CLASSES.len()], large: 0, bump: 0, end: 0 }));

fn class_of(size: usize) -> Option<usize> {
    CLASSES.iter().position(|&c| c >= size)
}

impl State {
    unsafe fn bump(&mut self, size: usize) -> *mut u8 {
        if self.bump == 0 {
            self.bump = wasm32::memory_size(0) * PAGE;
            self.end = self.bump;
        }
        let start = (self.bump + 15) & !15;
        let new_end = start + size;
        if new_end > self.end {
            let pages = (new_end - self.end).div_ceil(PAGE);
            if wasm32::memory_grow(0, pages) == usize::MAX {
                // The host's memory limit. An app cannot recover, so stop
                // with a reason; a null result would reach the panic
                // handler with none (and the aligned path below would use
                // it as an address).
                crate::trap_oom();
            }
            self.end += pages * PAGE;
        }
        self.bump = new_end;
        start as *mut u8
    }
}

unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let st = unsafe { &mut *self.0.get() };
        let size = layout.size().max(layout.align()).max(8);
        if layout.align() > 16 {
            // Rare: over-allocate and align. The block is never reused.
            let p = unsafe { st.bump(size + layout.align()) } as usize;
            return ((p + layout.align() - 1) & !(layout.align() - 1)) as *mut u8;
        }
        match class_of(size) {
            Some(c) => {
                let head = st.free[c];
                if head != 0 {
                    st.free[c] = unsafe { *(head as *const usize) };
                    head as *mut u8
                } else {
                    unsafe { st.bump(CLASSES[c]) }
                }
            }
            None => {
                let need = size.div_ceil(LARGE_UNIT) * LARGE_UNIT;
                // First fit, but not more than twice the size.
                let mut prev: *mut usize = &mut st.large;
                let mut cur = st.large;
                while cur != 0 {
                    let (bsize, next) = unsafe { (*(cur as *const usize), *((cur + 4) as *const usize)) };
                    if bsize >= need && bsize <= need * 2 {
                        unsafe { *prev = next };
                        return cur as *mut u8;
                    }
                    prev = (cur + 4) as *mut usize;
                    cur = next;
                }
                unsafe { st.bump(need) }
            }
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let st = unsafe { &mut *self.0.get() };
        if layout.align() > 16 {
            return;
        }
        let size = layout.size().max(layout.align()).max(8);
        let p = ptr as usize;
        match class_of(size) {
            Some(c) => {
                unsafe { *(p as *mut usize) = st.free[c] };
                st.free[c] = p;
            }
            None => {
                let bsize = size.div_ceil(LARGE_UNIT) * LARGE_UNIT;
                unsafe {
                    *(p as *mut usize) = bsize;
                    *((p + 4) as *mut usize) = st.large;
                }
                st.large = p;
            }
        }
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    // Formatting the message would link core::fmt into every app, so a
    // panic only traps. Runtime checks call `trap` with a message instead.
    wasm32::unreachable()
}
