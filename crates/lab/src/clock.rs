//! Virtual discrete-event clock owned by the Lab simulator.
//!
//! Advances directly between scheduled timestamps and `Core::next_due()` without wall-clock sleeps.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use monoagens_protocol::Time;

#[derive(Debug, Clone, Eq, PartialEq)]
struct QueueEntry<T> {
    at: Time,
    seq: u64,
    payload: T,
}

impl<T: Eq> Ord for QueueEntry<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.at
            .cmp(&other.at)
            .then_with(|| self.seq.cmp(&other.seq))
    }
}

impl<T: Eq> PartialOrd for QueueEntry<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Deterministic virtual clock with a `(Time, seq)` min-heap for scheduled events.
#[derive(Debug, Clone)]
pub struct VirtualClock<T> {
    now: Time,
    next_seq: u64,
    queue: BinaryHeap<Reverse<QueueEntry<T>>>,
}

impl<T: Eq> VirtualClock<T> {
    /// Creates a new virtual clock initialized at `start_time`.
    #[must_use]
    pub fn new(start_time: Time) -> Self {
        Self {
            now: start_time,
            next_seq: 0,
            queue: BinaryHeap::new(),
        }
    }

    /// Returns the current virtual timestamp.
    #[must_use]
    pub const fn now(&self) -> Time {
        self.now
    }

    /// Advances the virtual clock directly to `target` without sleeping.
    ///
    /// If `target` is earlier than `self.now()`, the clock clamps at `self.now()`.
    pub fn advance_to(&mut self, target: Time) {
        if target > self.now {
            self.now = target;
        }
    }

    /// Schedules `payload` at `at` (clamped to `>= self.now()`) and returns its deterministic tie-break seq.
    pub fn schedule_at(&mut self, at: Time, payload: T) -> u64 {
        let effective = if at < self.now { self.now } else { at };
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        self.queue.push(Reverse(QueueEntry {
            at: effective,
            seq,
            payload,
        }));
        seq
    }

    /// Schedules `payload` at `self.now().saturating_add_ms(delay_ms)`.
    pub fn schedule_after(&mut self, delay_ms: u64, payload: T) -> u64 {
        let at = self.now.saturating_add_ms(delay_ms);
        self.schedule_at(at, payload)
    }

    /// Returns the earliest timestamp currently queued, if any.
    #[must_use]
    pub fn next_time(&self) -> Option<Time> {
        self.queue.peek().map(|entry| entry.0.at)
    }

    /// Pops the earliest queued item in deterministic `(Time, seq)` order.
    pub fn pop_next(&mut self) -> Option<(Time, u64, T)> {
        self.queue
            .pop()
            .map(|Reverse(entry)| (entry.at, entry.seq, entry.payload))
    }

    /// Returns `true` if no events remain queued.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Returns the number of queued events.
    #[must_use]
    pub fn len(&self) -> usize {
        self.queue.len()
    }
}
