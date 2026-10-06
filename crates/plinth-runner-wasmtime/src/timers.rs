//! The `time` host API's timer queue (SPEC.md §8.4, §8.5).
//!
//! `TimerQueue` only tracks due times and ids; it does not sleep or spawn
//! threads. The desktop host drives it with `gpui`'s executor timers
//! (SPEC.md §9.4): it asks `next_deadline()`, sleeps until then, then calls
//! `due()` and sends a `timer` event (protocol `Event::Timer`) for each id
//! through `GuestPort::on_event`. Tests call `advance()` directly.

use std::collections::HashMap;
use std::time::{Duration, Instant};

struct Timer {
    period: Duration,
    due_at: Instant,
    repeat: bool,
}

/// A host's pending timers for one guest.
#[derive(Default)]
pub struct TimerQueue {
    next_id: u32,
    timers: HashMap<u32, Timer>,
}

impl TimerQueue {
    pub fn new() -> Self {
        Self { next_id: 0, timers: HashMap::new() }
    }

    /// Schedules a timer `ms` milliseconds from `now` and returns its id.
    pub fn set(&mut self, now: Instant, ms: u32, repeat: bool) -> u32 {
        self.next_id += 1;
        let id = self.next_id;
        let period = Duration::from_millis(ms as u64);
        self.timers.insert(id, Timer { period, due_at: now + period, repeat });
        id
    }

    /// Cancels a timer. Canceling an unknown id is not an error.
    pub fn cancel(&mut self, id: u32) {
        self.timers.remove(&id);
    }

    /// The ids due at or before `now`, oldest-scheduled first. Repeating
    /// timers are rescheduled for their next period; one-shot timers are
    /// removed.
    pub fn due(&mut self, now: Instant) -> Vec<u32> {
        let mut due: Vec<(Instant, u32)> =
            self.timers.iter().filter(|(_, t)| t.due_at <= now).map(|(id, t)| (t.due_at, *id)).collect();
        due.sort_by_key(|(at, _)| *at);
        for (_, id) in &due {
            if let Some(t) = self.timers.get_mut(id) {
                if t.repeat {
                    t.due_at += t.period;
                } else {
                    self.timers.remove(id);
                }
            }
        }
        due.into_iter().map(|(_, id)| id).collect()
    }

    /// The soonest due time, if any timer is pending.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.timers.values().map(|t| t.due_at).min()
    }

    pub fn is_empty(&self) -> bool {
        self.timers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_shot_fires_once() {
        let mut q = TimerQueue::new();
        let t0 = Instant::now();
        let id = q.set(t0, 10, false);
        assert_eq!(q.due(t0), Vec::<u32>::new());
        assert_eq!(q.due(t0 + Duration::from_millis(10)), vec![id]);
        assert_eq!(q.due(t0 + Duration::from_millis(20)), Vec::<u32>::new());
        assert!(q.is_empty());
    }

    #[test]
    fn repeating_reschedules() {
        let mut q = TimerQueue::new();
        let t0 = Instant::now();
        let id = q.set(t0, 10, true);
        assert_eq!(q.due(t0 + Duration::from_millis(10)), vec![id]);
        assert_eq!(q.due(t0 + Duration::from_millis(10)), Vec::<u32>::new());
        assert_eq!(q.due(t0 + Duration::from_millis(20)), vec![id]);
        assert!(!q.is_empty());
    }

    #[test]
    fn cancel_stops_future_firings() {
        let mut q = TimerQueue::new();
        let t0 = Instant::now();
        let id = q.set(t0, 10, true);
        q.cancel(id);
        assert_eq!(q.due(t0 + Duration::from_millis(100)), Vec::<u32>::new());
        assert!(q.is_empty());
    }

    #[test]
    fn canceling_unknown_timer_is_not_an_error() {
        let mut q = TimerQueue::new();
        q.cancel(999);
    }
}
