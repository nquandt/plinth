//! Global state for a single-threaded guest.
//!
//! The guest runs on one thread (SPEC.md §4.5), so a `RefCell` in a static
//! is safe. A conflicting borrow is a runtime bug and traps.

use core::cell::{Cell, RefCell};

pub struct Global<T>(RefCell<T>);

// SAFETY: a Plinth guest is single-threaded; the host never calls it from
// two threads at once.
unsafe impl<T> Sync for Global<T> {}

impl<T> Global<T> {
    pub const fn new(v: T) -> Self {
        Self(RefCell::new(v))
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        f(&mut self.0.borrow_mut())
    }

    pub fn read<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        f(&self.0.borrow())
    }
}

pub struct GlobalCell<T>(Cell<T>);

// SAFETY: as for `Global`.
unsafe impl<T> Sync for GlobalCell<T> {}

impl<T: Copy> GlobalCell<T> {
    pub const fn new(v: T) -> Self {
        Self(Cell::new(v))
    }

    pub fn get(&self) -> T {
        self.0.get()
    }

    pub fn set(&self, v: T) {
        self.0.set(v)
    }

    pub fn take(&self) -> T
    where
        T: Default,
    {
        self.0.take()
    }
}
