//! The async host-call request queue (SPEC.md §8.4, §8.5).
//!
//! Mirrors `TimerQueue`: a host API implementation (for example `dialog`)
//! calls `open()` to get a request id to return to the guest, and records
//! what answering it should look like. The actual UI (or other work) that
//! answers the request lives outside this queue; once it decides the
//! answer, the caller builds a `completion` event (protocol
//! `Event::Completion`) with this id and the result, and sends it through
//! `Guest::on_event`, exactly like a desktop host delivers a `timer` event.

use std::collections::HashSet;

/// A host's outstanding (not yet answered) requests for one guest. It only
/// tracks which ids are still open, so a caller can tell a stale or
/// double answer apart from a live one.
#[derive(Default)]
pub struct RequestQueue {
    next_id: u32,
    open: HashSet<u32>,
}

impl RequestQueue {
    pub fn new() -> Self {
        Self { next_id: 0, open: HashSet::new() }
    }

    /// Opens a new request and returns its id.
    pub fn open(&mut self) -> u32 {
        self.next_id += 1;
        self.open.insert(self.next_id);
        self.next_id
    }

    /// True if `id` is still open (not yet answered or dropped).
    pub fn is_open(&self, id: u32) -> bool {
        self.open.contains(&id)
    }

    /// Marks `id` as answered. Answering an unknown id is not an error.
    pub fn close(&mut self, id: u32) {
        self.open.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_open_until_closed() {
        let mut q = RequestQueue::new();
        let a = q.open();
        let b = q.open();
        assert_ne!(a, b);
        assert!(q.is_open(a));
        q.close(a);
        assert!(!q.is_open(a));
        assert!(q.is_open(b));
    }

    #[test]
    fn closing_unknown_id_is_not_an_error() {
        let mut q = RequestQueue::new();
        q.close(999);
    }
}
