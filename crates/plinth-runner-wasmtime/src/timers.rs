//! The `time` host API's timer queue (SPEC.md §8.4, §8.5).
//!
//! `TimerQueue` only tracks due times and ids; it does not sleep or spawn
//! threads. The desktop host drives it with `gpui`'s executor timers
//! (SPEC.md §9.4): it asks `next_deadline()`, sleeps until then, then calls
//! `due()` and sends a `timer` event (protocol `Event::Timer`) for each id
//! through `GuestPort::on_event`. Tests call `advance()` directly.
//!
//! A frame timer (core 1.12, `plinth:time` `onFrame`) has no due time: the
//! host fires every frame timer once per frame that it draws (`frames`).

use std::collections::HashMap;
use std::time::{Duration, Instant};

struct Timer {
    period: Duration,
    due_at: Instant,
    repeat: bool,
    /// A frame timer: fired by `frames`, never by `due`.
    frame: bool,
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
        self.timers.insert(id, Timer { period, due_at: now + period, repeat, frame: false });
        id
    }

    /// Starts a frame timer and returns its id (the same ids as `set`).
    pub fn set_frame(&mut self, now: Instant) -> u32 {
        self.next_id += 1;
        let id = self.next_id;
        self.timers.insert(id, Timer { period: Duration::ZERO, due_at: now, repeat: true, frame: true });
        id
    }

    /// The frame timers, oldest first.
    pub fn frames(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self.timers.iter().filter(|(_, t)| t.frame).map(|(id, _)| *id).collect();
        ids.sort_unstable();
        ids
    }

    /// Cancels a timer. Canceling an unknown id is not an error.
    pub fn cancel(&mut self, id: u32) {
        self.timers.remove(&id);
    }

    /// The ids due at or before `now`, oldest-scheduled first. Repeating
    /// timers are rescheduled for their next period; a repeating timer that
    /// is late by a whole period or more skips the missed firings (no burst
    /// of catch-up firings after a stall, as in a browser). One-shot timers
    /// are removed.
    pub fn due(&mut self, now: Instant) -> Vec<u32> {
        let mut due: Vec<(Instant, u32)> =
            self.timers.iter().filter(|(_, t)| !t.frame && t.due_at <= now).map(|(id, t)| (t.due_at, *id)).collect();
        due.sort_by_key(|(at, _)| *at);
        for (_, id) in &due {
            if let Some(t) = self.timers.get_mut(id) {
                if t.repeat {
                    t.due_at += t.period;
                    if t.due_at <= now {
                        t.due_at = now + t.period;
                    }
                } else {
                    self.timers.remove(id);
                }
            }
        }
        due.into_iter().map(|(_, id)| id).collect()
    }

    /// The soonest due time, if any timer is pending.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.timers.values().filter(|t| !t.frame).map(|t| t.due_at).min()
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
    fn a_late_repeating_timer_skips_missed_firings() {
        let mut q = TimerQueue::new();
        let t0 = Instant::now();
        let id = q.set(t0, 10, true);
        // A stall of 55 ms: one firing now, the next 10 ms later, no burst.
        let late = t0 + Duration::from_millis(55);
        assert_eq!(q.due(late), vec![id]);
        assert_eq!(q.due(late), Vec::<u32>::new());
        assert_eq!(q.next_deadline(), Some(late + Duration::from_millis(10)));
        // A timer that is a little late keeps its phase.
        assert_eq!(q.due(late + Duration::from_millis(12)), vec![id]);
        assert_eq!(q.next_deadline(), Some(late + Duration::from_millis(20)));
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
    fn frame_timers_fire_only_by_frames() {
        let mut q = TimerQueue::new();
        let t0 = Instant::now();
        let a = q.set(t0, 10, true);
        let f = q.set_frame(t0);
        assert_ne!(a, f, "one id space");
        assert_eq!(q.frames(), vec![f]);
        assert_eq!(q.next_deadline(), Some(t0 + Duration::from_millis(10)), "a frame timer has no deadline");
        assert_eq!(q.due(t0 + Duration::from_secs(1)), vec![a]);
        q.cancel(f);
        assert!(q.frames().is_empty());
    }

    #[test]
    fn canceling_unknown_timer_is_not_an_error() {
        let mut q = TimerQueue::new();
        q.cancel(999);
    }
}
